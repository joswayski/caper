package chat.caper.android

import android.Manifest
import android.app.Application
import android.content.ClipboardManager
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import androidx.core.content.ContextCompat
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import chat.caper.android.data.*
import chat.caper.android.model.*
import chat.caper.android.push.CaperNotifications
import chat.caper.android.push.ForegroundConversation
import chat.caper.android.push.PushActivity
import chat.caper.android.push.PushOffer
import chat.caper.android.push.PushRegistration
import chat.caper.android.push.pushOffer
import chat.caper.android.voice.VoiceCallService
import chat.caper.android.voice.VoiceState
import java.io.File
import java.io.IOException
import java.time.Instant
import java.util.UUID
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.flow.stateIn
import kotlinx.coroutines.launch
import kotlinx.coroutines.withTimeoutOrNull

class CaperViewModel(application: Application) : AndroidViewModel(application) {
    private val api = CaperApi()
    internal val whatsNew = WhatsNewClient(application, api)
    internal fun refreshWhatsNew() { viewModelScope.launch { whatsNew.refresh() } }
    private val tokens = TokenStore(application)
    private val mutable = MutableStateFlow(AppUiState())
    val state: StateFlow<AppUiState> = mutable.map { it.withAccountProfile() }
        .stateIn(viewModelScope, SharingStarted.Eagerly, mutable.value)
    private var accountToken: String? = null
    private var chatToken: String? = null
    private var chatAuthor: ChatAuthor? = null
    private var gateway: GatewayClient? = null
    /** The account-wide profiles feed: open while signed in, home and in the foreground. */
    private var profileFeed: ProfileFeedClient? = null
    private var profileFeedAccount = -1L
    private var gatewayStatus: Job? = null
    private var typingExpiry: Job? = null
    // Your own typing (web's setTyping): the wanted state, what the gateway last heard and when, and the idle stop.
    private var typingActive = false
    private var typingSent = false
    private var typingSentAt = 0L
    private var typingIdle: Job? = null
    private val typers = mutableMapOf<String, TypingAuthor>()
    private var generation = 0L
    private var historyAnchorRequest = 0L
    private var durableReplayCursor: String? = null
    private var refreshingHistory = false
    private var accountGeneration = 0L
    private var spaceAccessGeneration = 0L
    internal val accountEpoch: Long get() = accountGeneration
    internal val drafts = ConversationDrafts()
    internal val profileDrafts = ProfileMessageDrafts(api)
    internal val spaceAccessEpoch: Long get() = spaceAccessGeneration
    private var voiceAuthorizationRequest = 0L
    private val pendingSends = PendingSendTracker()
    private val unloadedReactions = mutableMapOf<String, ReactionUpdate>()
    private val unloadedAttachments = mutableMapOf<String, AttachmentsUpdate>()
    private data class ReactionIntent(val emoji: String, val active: Boolean, val version: Long)
    private val reactionIntents = mutableMapOf<String, LinkedHashMap<String, ReactionIntent>>()
    private val reactionWorkers = mutableMapOf<String, Job>()
    private val authoritativeReactionMessages = mutableMapOf<String, ChatMessage>()
    private val reactorCache = ReactorCache()
    private val pinSnapshots = mutableMapOf<String, ChatMessage>()
    private val forwardSnapshots = linkedMapOf<String, ChatMessage>()
    private val editSnapshots = mutableMapOf<String, ChatMessage>()
    private var pinSnapshotCursor: String? = null
    private val pinWorkers = mutableMapOf<String, Job>()
    private var reactionIntentVersion = 0L
    private var directRefresh: Job? = null
    private var voiceActivityRefresh: Job? = null
    private var peopleRefresh: Job? = null
    private var pendingDirectIntent: String? = null
    private var pendingChannelIntent: Pair<String, String>? = null
    /** A `feedback` notification tapped before Home was ready; opened once it is. */
    private var pendingFeedback: FeedbackTarget? = null
    /** Nonzero while [loadHome] has not yet chosen what to open, so a tap waits for it instead of racing it. */
    private var homeRouting = 0L
    private var homeRoutingRequests = 0L
    /** A report to focus (its message and thread) once its conversation opens the usual way. */
    private var pendingFocus: FeedbackTarget? = null
    /** The last visit opened, for Try again. */
    private var lastVisit: FeedbackTarget? = null
    private var feedbackRequest = 0L
    private var pendingInvitationIntent: Triple<String, String, String?>? = null
    private var notificationRouteRevision = 0L
    private val conversationHistories = linkedMapOf<String, ChatHistory>()
    private var conversationHistoryJob: Job? = null
    private var openingConversationId: String? = null
    private var conversationOpeningRevision = 0L
    private var openingForward = false
    private var notificationEdits: NotificationEdits? = null
    private var foreground = false
    private val uploader = AttachmentUploader(api)
    private val preparer by lazy { AttachmentPreparer(application) }
    private val pictureUploader = PictureUploader(api)
    /** Profile picture and space icon changes in flight, by [AppUiState.pictureSaves] key. */
    private val pictureJobs = mutableMapOf<String, Job>()
    /** Client compression settings from `GET /api/assets/usage`. */
    private var compression = CompressionSettings()
    private val uploadJobs = mutableMapOf<String, Job>()
    private val draftDirectories = mutableMapOf<String, File>()
    private val preparedDrafts = mutableMapOf<String, PreparedAttachment>()
    private var pendingNeedsUpload = false
    /** Drafts moved into the pending message; restored by Edit, deleted once confirmed. */
    private var pendingDrafts: List<DraftAttachmentUi> = emptyList()
    private val urlRefresh = AttachmentUrlRefresh()
    private val urlQueue = linkedSetOf<String>()
    private var urlBatch: Job? = null
    private var threadRequest = 0L
    /** `profile.updated` events received so far, and each person's latest with its count (see [repairProfiles]). */
    private var profileEventCount = 0L
    private val profileEvents = mutableMapOf<String, Pair<Long, ProfilePerson>>()
    private val uploadRoot = File(application.cacheDir, "attachment-uploads")
    private val uploadSession = File(uploadRoot, UUID.randomUUID().toString())
    private val threadPages = mutableMapOf<String, ThreadUi>()
    /** Web/Apple: the channel last opened in each space, in memory; returning to the space reopens it. */
    private val lastChannelBySpace = mutableMapOf<String, String>()
    /** Web's membershipRevision: a background reconcile never applies a snapshot taken while a change here ran. */
    private var membershipChanges = 0
    private var membershipRevision = 0L
    private var membershipRefresh: Job? = null
    private var unreadRevision = 0L
    private var unreadRefreshScheduled: Job? = null
    private val channelReads = mutableMapOf<String, String>()
    private var conversationVisible = true
    private var threadUnreadJob: Job? = null
    private var threadUnreadRevision = 0L
    private var threadUnreadDirty = false
    private val threadReadPending = mutableSetOf<String>()
    private var inviteLinkSequence = 0L
    /** An opened invite link that has not been shown yet; in memory only, so it never outlives the app. */
    private val pendingInviteLink = MutableStateFlow<InviteLinkRequest?>(null)
    /**
     * The opened invite link for "Join with invite link" to show, once someone with a finished
     * profile is home: a link opened while signed out or setting up a profile waits until then.
     */
    val inviteLinkToOpen: StateFlow<InviteLinkRequest?> =
        combine(pendingInviteLink, state) { link, current -> link?.takeIf { opensInviteLinks(current) } }
            .stateIn(viewModelScope, SharingStarted.Eagerly, null)

    init {
        loadHome()
        // Working copies only matter to this process; drop any left by an earlier one.
        viewModelScope.launch(Dispatchers.IO) { uploadRoot.listFiles()?.filter { it != uploadSession }?.forEach { it.deleteRecursively() } }
        // Signed URLs live 24-48 hours; an app left open for days refreshes them before expiry.
        viewModelScope.launch { while (true) { delay(10 * 60_000L); if (foreground) refreshExpiringUrls() } }
        viewModelScope.launch { state.map { it.selectedChannel?.id }.distinctUntilChanged().collect { publishForegroundConversation(); refreshThreadUnread() } }
        viewModelScope.launch { PushActivity.received.collect { scheduleUnreadRefresh() } }
    }

    private fun loadHome() {
        val requestAccountGeneration = accountGeneration
        val routing = ++homeRoutingRequests
        homeRouting = routing
        notificationEdits = null
        lastChannelBySpace.clear()
        profileEvents.clear()
        cancelConversationOpening()
        conversationHistories.clear()
        mutable.value = AppUiState(screen = SessionScreen.Loading, busy = true)
        viewModelScope.launch {
            try {
                val stored = tokens.read()
                val account = if (stored == null) null else try {
                    api.me(stored).also { accountToken = stored }
                } catch (error: ApiException) {
                    if (error.status == 401) { tokens.clear(); accountToken = null; null } else throw error
                }
                if (requestAccountGeneration != accountGeneration) return@launch
                if (account != null && (account.username == null || account.displayName == null)) {
                    mutable.value = AppUiState(screen = SessionScreen.Profile(account), account = account)
                    return@launch
                }
                if (account == null) {
                    mutable.value = AppUiState(screen = SessionScreen.SignedOut)
                    return@launch
                }
                val token = requireNotNull(accountToken)
                val list = api.spaces(token)
                val directs = runCatching { api.directConversations(token) }
                if (requestAccountGeneration != accountGeneration) return@launch
                mutable.value = AppUiState(
                    screen = SessionScreen.Home, restoring = true, account = account,
                    spaces = list.spaces, invitations = list.invitations, limits = list.limits,
                    directConversations = directs.getOrNull()?.conversations.orEmpty(),
                    error = directs.exceptionOrNull()?.let(::message),
                )
                viewModelScope.launch {
                    delay(RESTORE_LIMIT_MS)
                    if (requestAccountGeneration == accountGeneration) finishRestoring()
                }
                syncProfileFeed()
                refreshBlocks()
                refreshNotificationSettings()
                createChatSession(requestAccountGeneration)
                if (requestAccountGeneration != accountGeneration) return@launch
                checkUploads(requestAccountGeneration)
                startDirectRefresh()
                if (PushRegistration.enabled(getApplication())) viewModelScope.launch {
                    runCatching { PushRegistration.enable(getApplication()) }
                } else offerPush(account.id, requestAccountGeneration)
                val pending = pendingDirectIntent?.let { id -> mutable.value.directConversations.firstOrNull { it.id == id } }
                val pendingChannel = pendingChannelIntent?.takeIf { (space, _) -> list.spaces.any { it.id == space } }
                pendingChannelIntent = null
                val feedback = pendingFeedback?.takeIf { account.feedbackTeam }
                pendingFeedback = null
                if (homeRouting == routing) homeRouting = 0
                val invite = pendingInvitationIntent
                pendingInvitationIntent = null
                if (feedback != null) { pendingDirectIntent = null; openFeedbackConversation(feedback) }
                else if (invite != null) { finishRestoring(); openFromNotification(null, invite.second, invite.third, invite.first) }
                else if (pending != null) { pendingDirectIntent = null; selectDirect(pending) }
                else if (pendingChannel != null) selectSpace(pendingChannel.first, pendingChannel.second)
                else list.spaces.firstOrNull()?.let { selectSpace(it.id) } ?: finishRestoring()
            } catch (error: Throwable) {
                if (requestAccountGeneration == accountGeneration) {
                    mutable.value = AppUiState(screen = SessionScreen.SignedOut, error = message(error))
                }
            } finally {
                if (homeRouting == routing) homeRouting = 0
            }
        }
    }

    fun showLogin() { mutable.value = mutable.value.copy(screen = SessionScreen.SignedOut, error = null) }

    fun requestCode(email: String) = launchAccountAction { request ->
        val challenge = api.requestCode(email)
        if (request == accountGeneration) mutable.value = mutable.value.copy(
            screen = SessionScreen.Verify(challenge.challengeId, email, sentAt = android.os.SystemClock.elapsedRealtime()),
        )
    }

    /**
     * "Resend code" and "Email me a new code": a new code for the same email entry. The new
     * challenge replaces the old one (fresh attempts) and restarts the resend wait.
     */
    fun resendCode() = launchAccountAction { request ->
        val screen = mutable.value.screen as? SessionScreen.Verify ?: return@launchAccountAction
        if (screen.resends >= MAX_CODE_RESENDS) return@launchAccountAction
        val challenge = api.requestCode(screen.email)
        if (request == accountGeneration && mutable.value.screen == screen) mutable.value = mutable.value.copy(
            screen = SessionScreen.Verify(challenge.challengeId, screen.email, resends = screen.resends + 1, sentAt = android.os.SystemClock.elapsedRealtime()),
        )
    }

    fun verify(challenge: String, code: String) = launchAccountAction { request ->
        val result = try { api.verifyCode(challenge, code) } catch (error: ApiException) {
            val screen = mutable.value.screen
            if (request == accountGeneration && error.status == 401 && screen is SessionScreen.Verify && screen.challengeId == challenge)
                mutable.value = mutable.value.copy(screen = screen.copy(attemptsRemaining = error.attemptsRemaining))
            throw error
        }
        if (request != accountGeneration) return@launchAccountAction
        tokens.write(result.token)
        accountToken = result.token
        chatToken = null
        chatAuthor = null
        if (result.account.username == null || result.account.displayName == null) {
            mutable.value = AppUiState(screen = SessionScreen.Profile(result.account), account = result.account)
        } else loadHome()
    }

    fun saveProfile(username: String, displayName: String) = launchAccountAction { request ->
        val account = profileRequest { api.profile(requireAccountToken(), username, displayName) }
        if (request == accountGeneration) loadHome()
    }

    fun updateProfile(username: String, displayName: String, onSuccess: () -> Unit) = launchAction { request ->
        val saved = profileRequest { api.profile(requireAccountToken(), username, displayName) }
        if (request != accountGeneration) return@launchAction
        // Only `GET /api/account/me` reports the feedback team; a profile save keeps it.
        val account = saved.copy(feedbackTeam = saved.feedbackTeam || mutable.value.account?.feedbackTeam == true)
        mutable.value = mutable.value.copy(account = account, screen = SessionScreen.Home)
        chatToken = null
        chatAuthor = null
        mutable.value = mutable.value.copy(chatAuthorId = null)
        createChatSession(accountGeneration)
        onSuccess()
    }

    fun logout() {
        val token = accountToken
        VoiceCallService.stop(getApplication())
        ++accountGeneration
        ++spaceAccessGeneration
        invalidate()
        // Signing out gives up every conversation's staged files.
        mutable.value.attachmentDrafts.values.flatten().forEach { abandonDraft(it) }
        mutable.value = mutable.value.copy(attachmentDrafts = emptyMap(), attachmentErrors = emptyMap(), pictureSaves = emptyMap())
        pictureJobs.values.forEach { it.cancel() }; pictureJobs.clear()
        accountToken = null
        syncProfileFeed()
        chatToken = null
        chatAuthor = null
        channelReads.clear(); ++unreadRevision
        pendingFeedback = null; pendingFocus = null; lastVisit = null; ++feedbackRequest
        directRefresh?.cancel(); directRefresh = null
        voiceActivityRefresh?.cancel(); voiceActivityRefresh = null
        mutable.value = mutable.value.copy(voiceActivitySpaceIds = emptySet())
        peopleRefresh?.cancel(); peopleRefresh = null
        tokens.clear()
        drafts.clear()
        profileDrafts.clear().forEach { discardDraftFiles(it.key) }
        notificationEdits = null
        pendingInvitationIntent = null
        pendingChannelIntent = null
        pendingDirectIntent = null
        if (token != null) {
            viewModelScope.launch { PushRegistration.disable(getApplication(), token) }
            viewModelScope.launch { runCatching { api.logout(token) } }
        }
        loadHome()
    }

    /**
     * A notification tap: a DM by [conversationId], or a channel by [spaceId] and [channelId].
     * A `feedback` [kind] opens the visit view at [messageId] with the thread [threadRootId]
     * (or the message's own) open; invitations and message requests follow, then the rest.
     */
    fun openFromNotification(
        conversationId: String?, spaceId: String?, channelId: String?,
        kind: String? = null, messageId: String? = null, threadRootId: String? = null,
    ) {
        val id = Regex("^[A-Za-z0-9]{12}$")
        val routeRevision = ++notificationRouteRevision
        pendingInvitationIntent = null
        consumeNotificationInvitation()
        feedbackTargetFromNotification(kind, conversationId, spaceId, channelId, messageId, threadRootId)?.let { target ->
            val current = mutable.value
            val waiting = current.screen != SessionScreen.Home || current.account == null || homeRouting != 0L
            if (waiting || current.account?.feedbackTeam == true) {
                pendingDirectIntent = null; pendingChannelIntent = null
                if (waiting) pendingFeedback = target else openFeedbackConversation(target)
                return
            }
        }
        if (kind == "space.invitation" || kind == "channel.invitation") {
            if (conversationId != null || spaceId == null || !id.matches(spaceId) ||
                kind == "space.invitation" && channelId != null ||
                kind == "channel.invitation" && (channelId == null || !id.matches(channelId))) return
            pendingDirectIntent = null; pendingChannelIntent = null
            if (mutable.value.screen != SessionScreen.Home) {
                pendingInvitationIntent = Triple(kind, spaceId, channelId)
                return
            }
            val epoch = accountGeneration
            val navigation = generation
            val directNavigation = conversationOpeningRevision
            viewModelScope.launch {
                try {
                    val token = requireAccountToken()
                    val list = api.spaces(token)
                    if (epoch != accountGeneration || routeRevision != notificationRouteRevision ||
                        navigation != generation || directNavigation != conversationOpeningRevision) return@launch
                    mutable.value = mutable.value.copy(spaces = list.spaces, invitations = list.invitations, limits = list.limits)
                    if (kind == "space.invitation") {
                        val invitation = list.invitations.firstOrNull { it.id == spaceId }
                        mutable.value = mutable.value.copy(notificationInvitation = invitation,
                            error = if (invitation == null) "This invitation is no longer available." else null)
                    } else {
                        if (list.spaces.none { it.id == spaceId }) {
                            mutable.value = mutable.value.copy(error = "This invitation is no longer available.")
                            return@launch
                        }
                        val detail = api.space(token, spaceId)
                        if (epoch != accountGeneration || routeRevision != notificationRouteRevision ||
                            navigation != generation || directNavigation != conversationOpeningRevision) return@launch
                        if (detail.channelInvitations.none { it.channel.id == channelId }) {
                            mutable.value = mutable.value.copy(error = "This invitation is no longer available.")
                            return@launch
                        }
                        if (showsSpace(mutable.value, spaceId)) mutable.value = mutable.value.copy(selectedSpace = detail)
                        else selectSpace(spaceId)
                        mutable.value = mutable.value.copy(notificationBrowseSpace = spaceId)
                    }
                } catch (error: Throwable) {
                    if (epoch == accountGeneration && routeRevision == notificationRouteRevision && navigation == generation &&
                        directNavigation == conversationOpeningRevision) mutable.value = mutable.value.copy(error = message(error))
                }
            }
            return
        }
        if (conversationId != null) {
            if (!id.matches(conversationId)) return
            pendingChannelIntent = null
            mutable.value.directConversations.firstOrNull { it.id == conversationId }?.let(::selectDirect)
                ?: run { pendingDirectIntent = conversationId; refreshDirectConversations() }
            return
        }
        if (spaceId == null || channelId == null || !id.matches(spaceId) || !id.matches(channelId)) return
        pendingDirectIntent = null
        val current = mutable.value
        if (current.screen != SessionScreen.Home) { pendingChannelIntent = spaceId to channelId; return }
        if (current.selectedChannel?.id == channelId) return
        val loaded = current.selectedSpace?.takeIf { it.space.id == spaceId }?.channels?.firstOrNull { it.id == channelId }
        if (loaded != null) selectChannel(loaded) else selectSpace(spaceId, channelId)
    }

    /**
     * An invite link opened from another app (`https://caper.chat/invite/<code>` or
     * `caper://invite/<code>`): "Join with invite link" opens with it and loads its preview, now or
     * after sign-in and profile setup ([inviteLinkToOpen]). A newer link replaces one still
     * waiting; anything that is not an invite link is ignored.
     */
    fun openInviteLink(url: String) {
        val code = inviteCode(url) ?: return
        pendingInviteLink.value = InviteLinkRequest(url.trim(), code, ++inviteLinkSequence)
    }

    /** The dialog opened for [request]; a newer link that arrived meanwhile still waits. */
    fun inviteLinkShown(request: InviteLinkRequest) {
        pendingInviteLink.compareAndSet(request, null)
    }

    fun consumeNotificationInvitation() {
        mutable.value = mutable.value.copy(notificationInvitation = null, notificationBrowseSpace = null)
    }

    suspend fun canEnablePush(): Boolean {
        val token = accountToken ?: return false
        val epoch = accountGeneration
        return runCatching { "fcm" in api.pushConfig(token).platforms }.getOrDefault(false) && epoch == accountGeneration
    }

    /** Turns on push for this sign-in session; [done] gets the error to show, or null. */
    fun enablePush(done: (String?) -> Unit) {
        val request = accountGeneration
        val accountId = mutable.value.account?.id
        viewModelScope.launch {
            val error = try { PushRegistration.enable(getApplication()); null }
            catch (error: CancellationException) { throw error }
            catch (error: Throwable) { friendlyError(error, "Notifications could not be enabled.") }
            if (error == null && accountId != null) PushRegistration.setTurnedOff(getApplication(), accountId, false)
            if (request == accountGeneration) done(error)
        }
    }

    /** Turns push off on this phone and remembers it, so opening the app doesn't turn it back on. */
    fun disablePush(done: () -> Unit) {
        val token = accountToken
        mutable.value.account?.id?.let { PushRegistration.setTurnedOff(getApplication(), it, true) }
        viewModelScope.launch { PushRegistration.disable(getApplication(), token); done() }
    }

    /** Push is on by default (see [pushOffer]); this runs each time the app opens with an account. */
    private fun offerPush(accountId: String, request: Long) {
        val app = getApplication<Application>()
        if (!BuildConfig.FIREBASE_ENABLED || PushRegistration.turnedOff(app, accountId)) return
        viewModelScope.launch {
            val offered = canEnablePush()
            if (request != accountGeneration) return@launch
            val permitted = Build.VERSION.SDK_INT < 33 ||
                ContextCompat.checkSelfPermission(app, Manifest.permission.POST_NOTIFICATIONS) == PackageManager.PERMISSION_GRANTED
            when (pushOffer(BuildConfig.FIREBASE_ENABLED, PushRegistration.turnedOff(app, accountId), offered, permitted, PushRegistration.asked(app, accountId))) {
                PushOffer.ENABLE -> runCatching { PushRegistration.enable(app) }
                PushOffer.ASK -> mutable.value = mutable.value.copy(pushPrompt = true)
                PushOffer.NONE -> Unit
            }
        }
    }

    /** The activity is showing the prompt [offerPush] asked for. App open never asks this account again. */
    fun pushPromptShown() {
        mutable.value.account?.id?.let { PushRegistration.markAsked(getApplication(), it) }
        mutable.value = mutable.value.copy(pushPrompt = false)
    }

    /** The answer to that prompt, for the account that was signed in when it was shown ([epoch]). */
    fun pushPromptAnswered(granted: Boolean, epoch: Long) {
        if (granted && epoch == accountGeneration) enablePush { }
    }

    /**
     * Opens a space at [preferredChannelId] when it has one, otherwise the channel you last opened there, otherwise
     * its first joined channel; [notice] is shown once it opens. Choosing the space already shown keeps it.
     */
    fun selectSpace(id: String, preferredChannelId: String? = null, notice: String? = null) {
        cancelConversationOpening()
        if (mutable.value.spaces.none { it.id == id } || preferredChannelId == null && showsSpace(mutable.value, id)) return
        rememberConversationHistory()
        ++spaceAccessGeneration
        val request = ++generation
        closeChannel(clearPending = true)
        mutable.value = mutable.value.copy(deniedVoiceChannels = emptySet())
        mutable.value = mutable.value.copy(busy = true, error = null, openError = null, pendingSpaceInvitations = emptyList(), inviteLinks = emptyList())
        viewModelScope.launch {
            try {
                val since = profileEventCount
                val detail = api.space(requireAccountToken(), id)
                if (request != generation) return@launch
                val landing = spaceLandingChannel(detail.channels, preferredChannelId, lastChannelBySpace[id])
                // While restoring, the loading screen waits for the landing conversation too.
                mutable.value = mutable.value.copy(selectedSpace = detail, busy = false, presencePage = 0,
                    restoring = mutable.value.restoring && landing != null)
                // Before a cached conversation of this space opens.
                repairProfiles(detail.members.map { it.profile() }, since)
                landing?.let(::selectChannel)
                notice?.let { mutable.value = mutable.value.copy(error = it) }
            } catch (error: Throwable) {
                if (request == generation) {
                    if (error is ApiException && error.status == 404) removeUnavailableSpace(id)
                    else { retryOpen = { selectSpace(id, preferredChannelId) }; mutable.value = mutable.value.copy(busy = false, restoring = false, openError = message(error)) }
                }
            }
        }
    }

    private fun removeUnavailableSpace(id: String) {
        VoiceCallService.stopIfSpace(getApplication(), id)
        conversationHistories.entries.removeAll { it.value.space?.id == id }
        ++spaceAccessGeneration
        closeChannel(clearPending = true)
        mutable.value = mutable.value.copy(
            spaces = mutable.value.spaces.filter { it.id != id }, selectedSpace = null,
            busy = false, restoring = false, openError = null, error = "This space is no longer available.",
        )
    }

    private var retryOpen: (() -> Unit)? = null

    /** Web's Retry opening: repeats the space selection that failed. */
    fun retryOpening() { retryOpen?.invoke() }

    fun selectChannel(channel: Channel) {
        if (openingConversationId == channel.id) return
        cancelConversationOpening()
        // Choosing the conversation already shown keeps it, with its draft, scroll and live connection.
        if (showsChannel(mutable.value, channel)) {
            if (mutable.value.selectedChannel != channel) mutable.value = mutable.value.copy(selectedChannel = channel)
            return
        }
        val request = generation
        val accountRequest = accountGeneration
        val navigation = conversationOpeningRevision
        openingConversationId = channel.id
        mutable.value = mutable.value.copy(error = null, openError = null)
        // A first, uncached load still needs a skeleton. Otherwise keep the
        // displayed title, history, draft and gateway until the target is ready.
        if (mutable.value.selectedChannel == null && channel.id !in conversationHistories)
            mutable.value = mutable.value.copy(selectedChannel = channel, messagesLoading = true)
        conversationHistoryJob = viewModelScope.launch {
            try {
                // Fetch before installing history or allowing any automatic read acknowledgement.
                // A background badge refresh may already have replaced the cached unread snapshot.
                val visitUnread = if (!channel.direct) runCatching {
                    api.unreadSpaces(requireAccountToken()).spaces.flatMap { it.channels }
                        .firstOrNull { it.id == channel.id }
                }.getOrElse { if (it is CancellationException) throw it else null } else null
                val report = pendingFocus?.takeIf { it.channelId == channel.id }
                val history = if (report != null) api.history(accountToken, channel.id, around = report.thread)
                    else conversationHistories[channel.id] ?: api.history(accountToken, channel.id)
                if (request != generation || accountRequest != accountGeneration || navigation != conversationOpeningRevision) return@launch
                require(history.channel?.id == channel.id && history.space?.id == channel.spaceId) { "Caper returned another conversation." }
                rememberConversationHistory()
                openingConversationId = null
                conversationHistoryJob = null
                val committed = ++generation
                closeChannel(clearPending = true)
                if (!channel.direct) lastChannelBySpace[channel.spaceId] = channel.id
                mutable.value = mutable.value.copy(selectedChannel = channel, selectedDirectId = null, channelVisitUnread = visitUnread)
                renewDraftUploads(channel.id)
                installHistoryPins(history)
                mutable.value = mutable.value.copy(messages = mergeTimelinePins(history.messages), hasMoreMessages = history.hasMore, busy = false, messagesLoading = false, restoring = false)
                openGateway(channel.id, history.cursor, committed, channel.joined)
                applyPendingFocus(channel.id, history)
            } catch (error: CancellationException) {
                throw error
            } catch (error: Throwable) {
                if (request != generation || accountRequest != accountGeneration || navigation != conversationOpeningRevision) return@launch
                if (error is ApiException && error.status in listOf(401, 403, 404)) conversationHistories.remove(channel.id)
                if (mutable.value.selectedChannel?.id == channel.id && mutable.value.messagesLoading) {
                    if (error is ApiException && error.status in listOf(401, 403, 404)) { revokeChannel(); finishRestoring() }
                    else mutable.value = mutable.value.copy(busy = false, messagesLoading = false, restoring = false, messagesError = message(error))
                    return@launch
                }
                // A failed target must not revoke or clear the conversation still displayed.
                retryOpen = { selectChannel(channel) }
                mutable.value = mutable.value.copy(busy = false, restoring = false, openError = message(error))
            } finally {
                if (navigation == conversationOpeningRevision) { openingConversationId = null; conversationHistoryJob = null }
            }
        }
    }

    fun selectDirect(conversation: DirectConversation) {
        if (openingConversationId == conversation.id) return
        cancelConversationOpening()
        if (mutable.value.selectedDirectId == conversation.id && mutable.value.messagesError == null) return
        val request = generation
        val accountRequest = accountGeneration
        val navigation = conversationOpeningRevision
        openingConversationId = conversation.id
        mutable.value = mutable.value.copy(error = null)
        conversationHistoryJob = viewModelScope.launch {
            try {
                val report = pendingFocus?.takeIf { it.channelId == conversation.id }
                val history = if (report != null) api.history(requireAccountToken(), conversation.id, around = report.thread)
                    else conversationHistories[conversation.id] ?: api.history(requireAccountToken(), conversation.id)
                if (request != generation || accountRequest != accountGeneration || navigation != conversationOpeningRevision) return@launch
                require(history.space?.id == "" && history.channel?.id == conversation.id && history.channel?.direct == true) { "Caper returned another conversation." }
                rememberConversationHistory()
                openingConversationId = null
                conversationHistoryJob = null
                val committed = ++generation
                closeChannel(clearPending = true)
                val channel = Channel(conversation.id, "", conversation.peer.displayName, private = true, direct = true)
                mutable.value = mutable.value.copy(selectedChannel = channel, selectedDirectId = conversation.id)
                renewDraftUploads(channel.id)
                installHistoryPins(history)
                mutable.value = mutable.value.copy(messages = mergeTimelinePins(history.messages), hasMoreMessages = history.hasMore, busy = false, messagesLoading = false, restoring = false)
                openGateway(conversation.id, history.cursor, committed)
                refreshPeople()
                applyPendingFocus(conversation.id, history)
            } catch (error: CancellationException) {
                throw error
            } catch (error: Throwable) {
                if (request == generation && accountRequest == accountGeneration && navigation == conversationOpeningRevision) {
                    if (error is ApiException && error.status in listOf(401, 403, 404)) conversationHistories.remove(conversation.id)
                    mutable.value = mutable.value.copy(busy = false, restoring = false, error = message(error))
                }
            } finally {
                if (navigation == conversationOpeningRevision) { openingConversationId = null; conversationHistoryJob = null }
            }
        }
    }

    private fun cancelConversationOpening() {
        conversationOpeningRevision++
        if (openingConversationId != null) mutable.value = mutable.value.copy(busy = false)
        if (openingConversationId == mutable.value.selectedChannel?.id && mutable.value.messagesLoading)
            mutable.value = mutable.value.copy(selectedChannel = null, messagesLoading = false)
        openingConversationId = null
        conversationHistoryJob?.cancel(); conversationHistoryJob = null
    }

    private fun rememberConversationHistory() {
        val state = mutable.value
        val channel = state.selectedChannel ?: return
        val id = channel.id
        val cursor = durableReplayCursor ?: return
        val accessible = if (channel.direct) state.directConversations.any { it.id == id }
            else state.spaces.any { it.id == channel.spaceId } && state.selectedSpace?.channels?.any { it.id == id } == true
        if (!accessible || state.visit != null || state.messagesError != null || state.messagesLoading || state.contextStart != null || state.hasNewerMessages) {
            conversationHistories.remove(id)
            return
        }
        conversationHistories.remove(id)
        conversationHistories[id] = ChatHistory(
            messages = authoritativeMessages().filter { (it.threadRootId == null || it.broadcast) && it.id !in state.threadOnlyRows },
            cursor = cursor, hasMore = state.hasMoreMessages,
            pinnedMessages = pinSnapshots.values.filter { it.pin != null },
            space = ChatRoom(channel.spaceId, if (channel.direct) "Direct messages" else state.selectedSpace?.space?.name.orEmpty()),
            channel = ChatRoom(id, channel.name, direct = channel.direct),
        )
        while (conversationHistories.size > 20) conversationHistories.remove(conversationHistories.keys.first())
    }

    fun startDirect(username: String, done: () -> Unit = {}, failed: ((String) -> Unit)? = null) = launchAction { request ->
        try { openDirectByUsername(username, request, done) }
        catch (error: CancellationException) { throw error }
        catch (error: Throwable) {
            if (failed == null) throw error
            if (request == accountGeneration) failed(message(error))
        }
    }

    /** `POST /api/dms` by username (it returns an existing DM), then opens it unless you navigated meanwhile. */
    private suspend fun openDirectByUsername(username: String, request: Long, done: () -> Unit, fromProfile: Boolean = false) {
        val navigation = generation
        val directNavigation = conversationOpeningRevision
        val token = requireAccountToken()
        // Profile navigation previews even an uncached incoming request. Creating
        // an existing request would accept it before the user sees its controls.
        val existing = if (fromProfile) api.directConversations(token).conversations.firstOrNull {
            it.peer.username.equals(username, ignoreCase = true)
        } else null
        if (request != accountGeneration) return
        val conversation = existing ?: api.startDirectConversation(token, username)
        if (request != accountGeneration) return
        mutable.value = mutable.value.copy(directConversations = mergeDirects(mutable.value.directConversations, listOf(conversation)))
        if (navigation == generation && directNavigation == conversationOpeningRevision) { done(); selectDirect(conversation) }
    }

    /**
     * The mention card's Open conversation: opens a loaded DM with that person, otherwise the
     * same create-by-username flow, reporting failures to the card instead of the app.
     */
    fun messageMentioned(id: String?, username: String, done: () -> Unit, failed: (String) -> Unit) {
        val existing = mutable.value.directConversations.firstOrNull { if (id != null) it.peer.id == id else it.peer.username.equals(username, ignoreCase = true) }
        if (existing != null) {
            done()
            if (mutable.value.selectedDirectId != existing.id) selectDirect(existing)
            return
        }
        val request = accountGeneration
        viewModelScope.launch {
            try { openDirectByUsername(username, request, done, fromProfile = true) }
            catch (error: CancellationException) { throw error }
            catch (error: Throwable) { if (request == accountGeneration) failed(message(error)) }
        }
    }

    /** Only Send resolves/uploads a profile draft; opening happens after a confirmed message. */
    internal fun sendProfileMessage(person: String, id: String?, username: String, done: () -> Unit) {
        val token = accountToken ?: return
        val request = accountGeneration
        val navigation = generation
        val directNavigation = conversationOpeningRevision
        val files = profileDrafts[person].files
        viewModelScope.launch {
            val result = profileDrafts.send(person, id, username, token, session = {
                val capability = createChatSession(request) ?: error("Chat session is unavailable.")
                capability to requireNotNull(chatAuthor)
            }, directs = { mutable.value.directConversations }, blocked = { mutable.value.blockedIds })
            if (request != accountGeneration) return@launch
            if (result == null) {
                if (profileDrafts[person].error == directMessageError(DM_BLOCKED)) { refreshBlocks(); refreshDirectConversations() }
                return@launch
            }
            files.forEach { discardDraftFiles(it.key) }
            val (conversation, sent) = result
            mutable.value = mutable.value.copy(directConversations = mergeDirects(mutable.value.directConversations, listOf(conversation)))
            if (mutable.value.selectedChannel?.id == conversation.id) addMessage(sent)
            conversationHistories.remove(conversation.id)
            if (navigation == generation && directNavigation == conversationOpeningRevision) {
                done()
                selectDirect(conversation)
            }
        }
    }

    /** Opens the signed-in account's notes, creating the real DM on first use. */
    fun openSelfDirect() {
        val account = mutable.value.account ?: return
        mutable.value.directConversations.firstOrNull { it.peer.id == account.id }?.let(::selectDirect)
            ?: account.username?.let { startDirect(it) }
    }

    // --- Feedback (team members) -----------------------------------------------------------------

    /** Shows the Feedback inbox in place of the conversation and reloads its first page. */
    fun openFeedbackInbox() {
        if (mutable.value.account?.feedbackTeam != true) return
        cancelConversationOpening()
        rememberConversationHistory()
        pendingFocus = null
        ++generation
        closeChannel(clearPending = true)
        // The sidebar row always opens the inbox, not the archive.
        val inbox = mutable.value.feedback.let { if (it.archived) FeedbackInboxUi() else it }
        mutable.value = mutable.value.copy(feedbackShown = true, feedback = inbox, error = null, openError = null, busy = false)
        refreshFeedback()
    }

    /** The inbox's first page again (on open, pull to refresh, and after a reply in a visit). */
    fun refreshFeedback() = loadFeedback(first = true)

    fun loadMoreFeedback() = loadFeedback(first = false)

    /** The inbox, or archived threads ([archived]), from their first page. */
    fun showArchivedFeedback(archived: Boolean) {
        if (mutable.value.feedback.archived == archived) return
        mutable.value = mutable.value.copy(feedback = FeedbackInboxUi(archived = archived))
        refreshFeedback()
    }

    /**
     * Archives (or, from the archive, restores) [report]'s whole thread for the team. The
     * thread leaves the list at once; the list then reloads, keeping any error on show.
     */
    fun setFeedbackArchived(report: FeedbackReport, archived: Boolean) {
        val token = accountToken ?: return
        val account = accountGeneration
        val inbox = mutable.value.feedback
        // Drops any page in flight, which could still list the thread.
        ++feedbackRequest
        mutable.value = mutable.value.copy(feedback = inbox.copy(reports = withoutThread(inbox.reports, report), loading = false, loadingMore = false, error = null))
        viewModelScope.launch {
            val error = try { api.archiveFeedback(token, report.channel.id, report.threadRootId, archived); null }
            catch (cancelled: CancellationException) { throw cancelled }
            catch (failure: Throwable) { failure }
            if (account != accountGeneration) return@launch
            loadFeedback(first = true, notice = error?.let { message(it) })
        }
    }

    private fun loadFeedback(first: Boolean, notice: String? = null) {
        val token = accountToken ?: return
        val current = mutable.value
        if (current.account?.feedbackTeam != true) return
        val inbox = current.feedback
        val cursor = inbox.cursor
        val archived = inbox.archived
        if (!first && (cursor == null || inbox.loading || inbox.loadingMore)) return
        // A first page supersedes anything in flight; a next page waits for it.
        val request = ++feedbackRequest
        val account = accountGeneration
        mutable.value = current.copy(feedback = if (first) inbox.copy(loading = true, loadingMore = false, error = notice) else inbox.copy(loadingMore = true, error = null))
        viewModelScope.launch {
            try {
                val page = api.feedback(token, if (first) null else cursor, archived)
                if (account != accountGeneration || request != feedbackRequest || mutable.value.feedback.archived != archived) return@launch
                val shown = mutable.value.feedback
                mutable.value = mutable.value.copy(feedback = shown.copy(
                    reports = mergeFeedbackPage(shown.reports, page, first), cursor = validFeedbackCursor(page.cursor),
                    loaded = true, loading = false, loadingMore = false, error = notice,
                ))
            } catch (error: CancellationException) { throw error }
            catch (error: Throwable) {
                if (account != accountGeneration || request != feedbackRequest) return@launch
                if (error is ApiException && error.status == 404) {
                    // Not (or no longer) on the team: the inbox goes away.
                    mutable.value = mutable.value.copy(
                        account = mutable.value.account?.copy(feedbackTeam = false), feedback = FeedbackInboxUi(), feedbackShown = false,
                    )
                } else mutable.value = mutable.value.copy(feedback = mutable.value.feedback.copy(loading = false, loadingMore = false, error = notice ?: message(error)))
            }
        }
    }

    /** An inbox row: visit its conversation at the reported message. */
    fun openFeedbackReport(report: FeedbackReport) { feedbackTarget(report)?.let(::openFeedbackConversation) }

    /**
     * Opens [target]'s conversation by ID, even when it is in none of your lists. The server's
     * `visitor` flag means you are not in it: it opens as a visit, where you take part like a
     * member (typing, pins, voice in space channels) but have no member list, membership,
     * settings or read markers. A conversation you are in after all opens the usual
     * way when your lists have it. Only a reported reply opens its existing thread.
     */
    private fun openFeedbackConversation(target: FeedbackTarget) {
        val token = accountToken ?: return
        cancelConversationOpening()
        rememberConversationHistory()
        lastVisit = target
        pendingFocus = null
        val request = ++generation
        val accountRequest = accountGeneration
        closeChannel(clearPending = true)
        val direct = target.spaceId == null
        mutable.value = mutable.value.copy(
            selectedChannel = Channel(target.channelId, target.spaceId.orEmpty(), target.channelName.orEmpty(), private = true, direct = direct),
            visit = VisitUi(target.channelId, target.spaceId, target.spaceName, target.channelName.orEmpty(), direct, reporter = target.reporter),
            messagesLoading = true, busy = false, error = null, openError = null,
        )
        viewModelScope.launch {
            try {
                val history = api.history(token, target.channelId, around = target.thread)
                if (request != generation || accountRequest != accountGeneration) return@launch
                val room = requireNotNull(history.channel) { "Conversation is unavailable." }
                require(room.id == target.channelId) { "Caper returned another conversation." }
                val spaceId = history.space?.id?.takeIf { it.isNotEmpty() }
                if (!history.visitor && openAsMember(target, room.direct, spaceId)) return@launch
                val visit = VisitUi(target.channelId, spaceId, history.space?.name?.takeIf { spaceId != null }, room.name, room.direct,
                    visitor = history.visitor, reporter = target.reporter)
                val channel = Channel(target.channelId, spaceId.orEmpty(), room.name, private = true, direct = room.direct)
                installHistoryPins(history)
                mutable.value = mutable.value.copy(
                    selectedChannel = channel, visit = visit,
                    messages = mergeTimelinePins(history.messages), hasMoreMessages = history.hasMore,
                    busy = false, messagesLoading = false, messagesError = null, restoring = false,
                )
                // Visitors take part like members: live updates, typing and (space channels) voice rosters.
                openGateway(channel.id, history.cursor, request)
                focusReport(target, history)
            } catch (error: CancellationException) { throw error }
            catch (error: Throwable) {
                if (request != generation || accountRequest != accountGeneration) return@launch
                if (error is ApiException && error.status in listOf(401, 403, 404)) {
                    ++generation
                    closeChannel(clearPending = true)
                    mutable.value = mutable.value.copy(feedbackShown = mutable.value.account?.feedbackTeam == true, restoring = false,
                        error = "This conversation is no longer available.")
                } else mutable.value = mutable.value.copy(busy = false, messagesLoading = false, restoring = false, messagesError = message(error))
            }
        }
    }

    /** A reported conversation you are in: open it from your lists, then focus the report. False when they lack it. */
    private fun openAsMember(target: FeedbackTarget, direct: Boolean, spaceId: String?): Boolean {
        val current = mutable.value
        if (direct) {
            val conversation = current.directConversations.firstOrNull { it.id == target.channelId } ?: return false
            pendingFocus = target
            selectDirect(conversation)
            return true
        }
        if (spaceId == null || current.spaces.none { it.id == spaceId }) return false
        pendingFocus = target
        selectSpace(spaceId, target.channelId)
        return true
    }

    /** Focuses a report waiting for [channelId] to open (see [openAsMember]); any other conversation drops it. */
    private fun applyPendingFocus(channelId: String, history: ChatHistory) {
        val target = pendingFocus ?: return
        pendingFocus = null
        if (target.channelId == channelId) focusReport(target, history)
    }

    /** Root reports stay in the conversation; only reported replies open a thread. */
    private fun focusReport(target: FeedbackTarget, history: ChatHistory) {
        mutable.value = mutable.value.copy(
            contextStart = history.messages.firstOrNull()?.seq,
            contextEnd = if (history.hasNewer) history.messages.lastOrNull()?.seq else null,
            hasNewerMessages = history.hasNewer,
        )
        if (target.replyRoot != null) {
            goToMessage(target.channelId, target.messageId, target.replyRoot) {}
        } else if (history.messages.any { it.id == target.messageId }) {
            mutable.value = mutable.value.copy(thread = null, focusedMessageId = target.messageId,
                focusRevision = mutable.value.focusRevision + 1)
        } else {
            mutable.value = mutable.value.copy(messagesError = "Message context is unavailable.")
        }
    }

    /** Back from a visit: the Feedback inbox, refreshed. */
    fun closeVisit() {
        if (mutable.value.visit == null) return
        pendingFocus = null
        ++generation
        closeChannel(clearPending = true)
        if (mutable.value.account?.feedbackTeam == true) {
            mutable.value = mutable.value.copy(feedbackShown = true)
            refreshFeedback()
        }
    }

    /**
     * Refreshes `/api/people` for DM mentions and the space-invite and new-DM username
     * suggestions; the previous list (or the DM peer) stays until it succeeds.
     */
    fun refreshPeople() {
        val token = accountToken ?: return
        val request = accountGeneration
        peopleRefresh?.cancel()
        peopleRefresh = viewModelScope.launch {
            val since = profileEventCount
            runCatching { api.people(token) }.onSuccess { result ->
                if (request == accountGeneration) {
                    mutable.value = mutable.value.copy(people = result.people)
                    repairProfiles(result.people.map { it.profile() }, since)
                }
            }
        }
    }

    fun refreshDirectConversations() {
        val token = accountToken ?: return
        val request = accountGeneration
        viewModelScope.launch {
            val since = profileEventCount
            runCatching { api.directConversations(token) }.onSuccess { result ->
                if (request == accountGeneration) {
                    mutable.value = mutable.value.copy(directConversations = result.conversations)
                    repairProfiles(result.conversations.map { it.peer.profile() }, since)
                    pendingDirectIntent?.let { id -> result.conversations.firstOrNull { it.id == id } }?.let {
                        pendingDirectIntent = null
                        selectDirect(it)
                    }
                }
            }
        }
    }

    /** A tap on "Message requests": the explicit choice wins over following the open request. */
    fun setRequestsOpen(open: Boolean) { mutable.value = mutable.value.copy(requestsOpen = open) }

    /** Accept: the request joins the main list and the composer replaces the request bar. */
    fun acceptRequest(conversation: DirectConversation, failed: (String) -> Unit) = accountRequest(failed) { token, request ->
        val accepted = api.acceptDirectRequest(token, conversation.id)
        if (request != accountGeneration) return@accountRequest
        val directs = mutable.value.directConversations.map { if (it.id == conversation.id) accepted.copy(lastSeq = maxSeq(it.lastSeq, accepted.lastSeq)) else it }
        mutable.value = mutable.value.copy(directConversations = directs, requestsOpen = mutable.value.requestsOpen.takeIf { messageRequests(directs).isNotEmpty() })
    }

    /** Decline hides the request from you only; the sender is not told. */
    fun declineRequest(conversation: DirectConversation, done: () -> Unit, failed: (String) -> Unit) = accountRequest(failed) { token, request ->
        api.declineDirectRequest(token, conversation.id)
        if (request != accountGeneration) return@accountRequest
        removeRequest(conversation.id)
        done()
    }

    /** Drops a declined (or blocked) request and leaves it if it is open, back toward the requests list. */
    private fun removeRequest(id: String) {
        conversationHistories.remove(id)
        if (openingConversationId == id) cancelConversationOpening()
        val directs = mutable.value.directConversations.filter { it.id != id }
        mutable.value = mutable.value.copy(directConversations = directs, requestsOpen = if (messageRequests(directs).isNotEmpty()) true else null)
        if (mutable.value.selectedDirectId != id) return
        val channel = mutable.value.selectedSpace?.channels?.firstOrNull { it.joined }
        if (channel != null) selectChannel(channel) else invalidate()
    }

    fun refreshBlocks() {
        val token = accountToken ?: return
        val request = accountGeneration
        viewModelScope.launch {
            try {
                val since = profileEventCount
                val blocks = api.blocks(token).blocks
                if (request == accountGeneration) {
                    mutable.value = mutable.value.copy(blocks = blocks, blocksError = null, blocksLoaded = true)
                    repairProfiles(blocks.map { it.profile() }, since)
                }
            } catch (error: CancellationException) { throw error }
            catch (error: Throwable) { if (request == accountGeneration) mutable.value = mutable.value.copy(blocksError = message(error)) }
        }
    }

    /** Who a message's author is, for blocking: the best name this client already has. */
    fun blockTarget(author: ChatAuthor): BlockedAccount {
        val card = authorCard(author, mutable.value)
        return BlockedAccount(author.id, card.username.orEmpty(), card.displayName ?: author.name,
            card.avatarId, card.avatarAsset, card.avatarUrl, card.admin, card.team)
    }

    /** Blocks [account] everywhere you share; a pending request from them is declined too. */
    fun block(account: BlockedAccount, done: () -> Unit, failed: (String) -> Unit) = accountRequest(failed) { token, request ->
        require(account.canBlock) { "Caper admins and the Caper team can't be blocked." }
        api.block(token, account.id)
        if (request != accountGeneration) return@accountRequest
        val current = mutable.value
        val declined = current.directConversations.filter { it.peer.id == account.id && it.incoming }.map { it.id }
        mutable.value = current.copy(
            blocks = listOf(account) + current.blocks.filter { it.id != account.id },
            directConversations = current.directConversations.map { if (it.peer.id == account.id) it.copy(blocked = true) else it },
            typingAuthors = current.typingAuthors.filter { it.id != account.id },
        )
        declined.forEach(::removeRequest)
        done()
        refreshBlocks()
    }

    fun unblock(accountId: String, failed: (String) -> Unit = { mutable.value = mutable.value.copy(error = it) }) = accountRequest(failed) { token, request ->
        api.unblock(token, accountId)
        if (request != accountGeneration) return@accountRequest
        mutable.value = mutable.value.copy(
            blocks = mutable.value.blocks.filter { it.id != accountId },
            directConversations = mutable.value.directConversations.map { if (it.peer.id == accountId) it.copy(blocked = false) else it },
        )
        refreshBlocks()
    }

    suspend fun directPrivacy(): String = api.directPrivacy(requireAccountToken()).directMessages

    suspend fun setDirectPrivacy(value: String): String {
        require(directPrivacyOptions.any { it.first == value }) { "Unknown privacy setting." }
        return api.setDirectPrivacy(requireAccountToken(), value).directMessages
    }

    /**
     * The DM or channel on screen while the app is in the foreground: its pushes show
     * nothing, and its notification is cleared.
     */
    private fun publishForegroundConversation() {
        val id = mutable.value.selectedChannel?.id.takeIf { foreground }
        ForegroundConversation.id = id
        if (id != null) runCatching { CaperNotifications.cancel(getApplication(), id) }
    }

    /** Loads notification settings after sign-in and when settings or menus open. */
    fun refreshNotificationSettings() {
        val token = accountToken ?: return
        val request = accountGeneration
        viewModelScope.launch {
            try {
                val settings = api.notificationSettings(token)
                if (request != accountGeneration) return@launch
                val edits = notificationEdits?.apply { loaded(settings) } ?: NotificationEdits(settings).also { notificationEdits = it }
                mutable.value = mutable.value.copy(notificationSettings = edits.shown, notificationSettingsError = null)
            } catch (error: CancellationException) { throw error }
            catch (error: Throwable) {
                if (request == accountGeneration && mutable.value.notificationSettings == null)
                    mutable.value = mutable.value.copy(notificationSettingsError = "Couldn’t load notification settings.")
            }
        }
    }

    fun setNotificationLevel(level: String) = editNotifications(ACCOUNT_LEVEL_KEY, { it.copy(level = level) }) { token ->
        val saved = api.updateNotificationSettings(token, level = level).level
        return@editNotifications { it.copy(level = saved) }
    }

    fun setMobileNotifications(mobile: String) = editNotifications(MOBILE_KEY, { it.copy(mobile = mobile) }) { token ->
        val saved = api.updateNotificationSettings(token, mobile = mobile).mobile
        return@editNotifications { it.copy(mobile = saved) }
    }

    fun setSpaceNotifications(spaceId: String, change: OverrideChange) =
        editOverride(NotificationOverride(spaceId = spaceId), change) { api.setSpaceNotifications(it, spaceId, change) }

    fun setChannelNotifications(spaceId: String, channelId: String, change: OverrideChange) =
        editOverride(NotificationOverride(spaceId = spaceId, channelId = channelId), change) { api.setChannelNotifications(it, spaceId, channelId, change) }

    fun setDirectNotifications(conversationId: String, change: OverrideChange) =
        editOverride(NotificationOverride(conversationId = conversationId), change) { api.setDirectNotifications(it, conversationId, change) }

    fun dismissNotificationError(key: String) { mutable.value = mutable.value.copy(notificationErrors = mutable.value.notificationErrors - key) }

    private fun editOverride(scope: NotificationOverride, change: OverrideChange, save: suspend (String) -> NotificationOverride) {
        val key = scope.key
        editNotifications(key, { it.withOverride(key, (it.override(key) ?: scope).applying(change)) }) { token ->
            val saved = save(token)
            return@editNotifications { it.withOverride(key, saved) }
        }
    }

    /** Shows [edit] at once, saves it, and puts [key] back with a short error when the save fails. */
    private fun editNotifications(
        key: String,
        edit: (NotificationSettings) -> NotificationSettings,
        save: suspend (String) -> (NotificationSettings) -> NotificationSettings,
    ) {
        val token = accountToken ?: return
        val edits = notificationEdits ?: return
        val request = accountGeneration
        val ticket = edits.begin(key, edit)
        mutable.value = mutable.value.copy(notificationSettings = edits.shown, notificationErrors = mutable.value.notificationErrors - key)
        viewModelScope.launch {
            try {
                val answer = save(token)
                if (request != accountGeneration) return@launch
                edits.succeeded(key, ticket, answer)
                mutable.value = mutable.value.copy(notificationSettings = edits.shown)
            } catch (error: CancellationException) { throw error }
            catch (error: Throwable) {
                if (request != accountGeneration) return@launch
                edits.failed(key, ticket)
                mutable.value = mutable.value.copy(notificationSettings = edits.shown, notificationErrors = mutable.value.notificationErrors + (key to NOTIFICATION_SAVE_ERROR))
            }
        }
    }

    private fun accountRequest(failed: (String) -> Unit, block: suspend (String, Long) -> Unit) = viewModelScope.launch {
        val request = accountGeneration
        try { block(requireAccountToken(), request) }
        catch (error: CancellationException) { throw error }
        catch (error: Throwable) { if (request == accountGeneration) failed(message(error)) }
    }

    private fun startDirectRefresh() {
        directRefresh?.cancel()
        refreshUnreadSpaces()
        refreshVoiceActivity()
        directRefresh = viewModelScope.launch { while (true) { delay(15_000); refreshVoiceActivity(); refreshDirectConversations(); refreshUnreadSpaces(); refreshMembership() } }
    }

    private fun refreshVoiceActivity() {
        val token = accountToken ?: return
        if (voiceActivityRefresh?.isActive == true) return
        val account = accountGeneration
        voiceActivityRefresh = viewModelScope.launch {
            try {
                val snapshot = api.spaceVoiceActivity(token)
                if (account == accountGeneration) mutable.value = mutable.value.copy(voiceActivitySpaceIds = snapshot.spaceIds.toSet())
            } catch (error: CancellationException) { throw error }
            catch (_: Throwable) {
                if (account == accountGeneration) mutable.value = mutable.value.copy(voiceActivitySpaceIds = emptySet())
            }
        }
    }

    /** Pushes refresh unread state between polls; a burst coalesces into one request about a second after the first. */
    private fun scheduleUnreadRefresh() {
        if (accountToken == null || unreadRefreshScheduled?.isActive == true) return
        unreadRefreshScheduled = viewModelScope.launch { delay(1_000); refreshUnreadSpaces() }
    }

    private fun refreshUnreadSpaces() {
        refreshThreadUnread()
        val token = accountToken ?: return
        val account = accountGeneration; val revision = ++unreadRevision
        viewModelScope.launch {
            runCatching { api.unreadSpaces(token) }.onSuccess {
                if (account == accountGeneration && revision == unreadRevision) {
                    mutable.value = mutable.value.copy(unreadSpaces = it.spaces, feedbackWaiting = it.feedbackWaiting)
                    if (!mutable.value.messagesLoading) mutable.value.selectedChannel?.id?.let { id -> markConversationRead(id, durableReplayCursor) }
                }
            }
        }
    }

    private fun refreshThreadUnread() {
        threadUnreadDirty = true
        if (threadUnreadJob?.isActive == true) return
        threadUnreadJob = viewModelScope.launch {
            delay(300)
            while (threadUnreadDirty) {
                threadUnreadDirty = false
                val token = accountToken ?: break
                val channel = mutable.value.selectedChannel?.id ?: break
                val account = accountGeneration
                val revision = threadUnreadRevision
                runCatching { api.unreadThreads(token, channel) }.onSuccess { result ->
                    if (account == accountGeneration && channel == mutable.value.selectedChannel?.id && revision == threadUnreadRevision)
                        mutable.value = mutable.value.copy(unreadThreads = result.threads.associate { it.rootId to it.seq })
                }
            }
        }
    }

    /** Called only when a reply's bottom is visible, never for loaded/prefetched history. */
    fun threadReplyVisible(channel: String, root: String, seq: String) {
        val current = mutable.value
        val thread = current.thread ?: return
        if (!foreground || !conversationVisible || current.screen != SessionScreen.Home || current.visit != null ||
            current.selectedChannel?.id != channel || thread.rootId != root || thread.loading || thread.hasNewer) return
        val unread = current.unreadThreads[root] ?: return
        if (seq.toBigInteger() < unread.toBigInteger()) return
        val token = accountToken ?: return
        val account = accountGeneration
        val revision = threadUnreadRevision
        val key = "$account:$channel:$root"
        if (!threadReadPending.add(key)) return
        viewModelScope.launch {
            try {
                api.markThreadRead(token, channel, root, seq)
                if (account == accountGeneration && channel == mutable.value.selectedChannel?.id && revision == threadUnreadRevision) {
                    ++threadUnreadRevision
                    mutable.value = mutable.value.copy(unreadThreads = mutable.value.unreadThreads.filter { (id, latest) ->
                        id != root || latest.toBigInteger() > seq.toBigInteger()
                    })
                    refreshThreadUnread()
                }
            } catch (error: CancellationException) { throw error }
            catch (_: Throwable) { /* The visible-row callback retries; never clear on failure. */ }
            finally { threadReadPending.remove(key) }
        }
    }

    fun setConversationVisible(visible: Boolean) {
        conversationVisible = visible
        if (visible && !mutable.value.messagesLoading) mutable.value.selectedChannel?.id?.let { markConversationRead(it, durableReplayCursor) }
    }

    private fun markConversationRead(id: String, seq: String?) {
        if (seq == null) return
        if (openingConversationId == id) return
        val current = mutable.value
        // A visit never marks anything read: the team member is not in the conversation.
        if (current.visit != null) return
        if (!foreground || !conversationVisible || current.selectedChannel?.id != id || current.selectedDirect?.incoming == true) return
        // An older window (jumped to a reply or search result) is not the latest, for channels and DMs alike.
        if (current.hasNewerMessages) return
        if (current.selectedDirectId == id) { markDirectRead(id, seq); return }
        val space = current.selectedSpace?.space?.id?.takeIf { it.isNotEmpty() } ?: return
        val token = accountToken ?: return
        if ((channelReads[id] ?: "0").toBigInteger() >= seq.toBigInteger()) return
        val account = accountGeneration
        viewModelScope.launch {
            runCatching { api.markChannelRead(token, space, id, seq) }.onSuccess {
                if (account != accountGeneration) return@onSuccess
                ++unreadRevision
                channelReads[id] = maxOf((channelReads[id] ?: "0").toBigInteger(), seq.toBigInteger()).toString()
                mutable.value = mutable.value.copy(unreadSpaces = mutable.value.unreadSpaces.map { entry ->
                    if (entry.id != space) entry else entry.copy(channels = entry.channels.filter { it.id != id || it.lastSeq.toBigInteger() > seq.toBigInteger() })
                }.filter { it.channels.isNotEmpty() })
            }
        }
    }

    /**
     * Web's membership reconcile, every 15 seconds and on returning to the app: invitations, and spaces or channels
     * changed elsewhere, appear without a restart. A change made here meanwhile wins over the older snapshot, and a
     * failed read keeps what is shown: an outage is not revocation.
     */
    private fun refreshMembership() {
        val token = accountToken ?: return
        val start = mutable.value
        if (!foreground || start.screen != SessionScreen.Home || start.busy || membershipChanges > 0 || membershipRefresh?.isActive == true) return
        val account = accountGeneration
        val revision = membershipRevision
        fun unchanged() = account == accountGeneration && revision == membershipRevision && membershipChanges == 0
        membershipRefresh = viewModelScope.launch {
            try {
                val list = api.spaces(token)
                val known = mutable.value
                if (!unchanged() || known.spaces != start.spaces || known.invitations != start.invitations) return@launch
                val available = list.spaces.map { it.id }.toSet()
                lastChannelBySpace.keys.retainAll(available)
                conversationHistories.entries.removeAll { it.value.space?.id?.let { id -> id.isNotEmpty() && id !in available } == true }
                mutable.value = known.copy(spaces = list.spaces, invitations = list.invitations, limits = list.limits)
                val open = known.selectedSpace?.space?.id ?: return@launch
                if (open !in available) {
                    // Removed from the open space elsewhere, or it was deleted.
                    val notice = "This space is no longer available."
                    // A DM, a visit or the Feedback inbox stays open; only the sidebar loses the space.
                    if (known.selectedDirectId != null || known.visit != null || known.feedbackShown) {
                        VoiceCallService.stopIfSpace(getApplication(), open)
                        ++spaceAccessGeneration
                        mutable.value = mutable.value.copy(selectedSpace = null, error = notice)
                    } else {
                        ++generation
                        removeUnavailableSpace(open)
                        list.spaces.firstOrNull()?.let { selectSpace(it.id, notice = notice) }
                    }
                    return@launch
                }
                if (known.selectedDirectId != null || known.visit != null || known.feedbackShown) return@launch
                val navigation = generation
                val since = profileEventCount
                val detail = api.space(token, open)
                val current = mutable.value
                if (!unchanged() || navigation != generation || current.busy || current.selectedSpace?.space?.id != open || current.selectedDirectId != null ||
                    current.visit != null || current.feedbackShown || detail == current.selectedSpace) return@launch
                replaceDetail(detail)
                repairProfiles(detail.members.map { it.profile() }, since)
                val selected = current.selectedChannel
                val retained = selected?.let { channel -> detail.channels.firstOrNull { it.id == channel.id } }
                if (selected != null && retained != null) {
                    // Joined or left elsewhere: reopen with or without the composer. Otherwise only its name or privacy changed.
                    if (retained.joined != selected.joined) selectChannel(retained) else mutable.value = mutable.value.copy(selectedChannel = retained)
                } else {
                    // Deleted, or private access removed, elsewhere (or nothing was open): the first joined channel, as web.
                    selected?.let { VoiceCallService.stopIfChannel(getApplication(), it.id) }
                    val next = detail.channels.firstOrNull { it.joined }
                    if (next != null) selectChannel(next) else if (selected != null) { ++generation; closeChannel(clearPending = true) }
                }
            } catch (error: CancellationException) { throw error }
            catch (_: Throwable) { /* An outage is not revocation; keep known navigation. */ }
        }
    }

    fun setForeground(active: Boolean) {
        foreground = active
        publishForegroundConversation()
        syncProfileFeed()
        if (!active) return
        refreshDirectConversations()
        refreshExpiringUrls()
        mutable.value.selectedChannel?.id?.let { renewDraftUploads(it) }
        mutable.value.pendingMessage?.takeIf { it.uploadFailed }?.let { send(it.text, threadRootId = it.threadRootId, broadcast = it.broadcast) }
        refreshMembership()
        refreshUnreadSpaces()
        refreshVoiceActivity()
        val current = mutable.value
        if (!current.messagesLoading) current.selectedChannel?.id?.let { markConversationRead(it, durableReplayCursor) }
    }

    private fun markDirectRead(id: String, seq: String) {
        if (!foreground) return
        val token = accountToken ?: return
        val request = accountGeneration
        viewModelScope.launch {
            runCatching { api.markDirectConversationRead(token, id, seq) }.onSuccess {
                if (request == accountGeneration) mutable.value = mutable.value.copy(directConversations = mutable.value.directConversations.map {
                    if (it.id == id) it.copy(
                        readSeq = maxOf(it.readSeq.toBigInteger(), seq.toBigInteger()).toString(),
                        lastSeq = maxOf(it.lastSeq.toBigInteger(), seq.toBigInteger()).toString(),
                    ) else it
                })
            }
        }
    }

    fun openVoiceChannel(voice: VoiceState, onOpened: () -> Unit = {}) {
        val spaceId = voice.spaceId ?: return
        val channelId = voice.channelId ?: return
        val context = VoiceNavigationContext(generation, accountGeneration, VoiceCallService.navigationEpoch(), spaceId, channelId)
        if (!context.isCurrent(generation, accountGeneration, VoiceCallService.navigationEpoch(), VoiceCallService.state.value)) return
        if (mutable.value.selectedSpace?.space?.id == spaceId && mutable.value.selectedChannel?.id == channelId ||
            mutable.value.visit?.channelId == channelId) {
            onOpened()
            return
        }
        // A call in a visited channel: visit it again.
        if (mutable.value.spaces.none { it.id == spaceId }) {
            lastVisit?.takeIf { it.channelId == channelId && it.spaceId == spaceId }?.let { openFeedbackConversation(it); onOpened() }
            return
        }
        val token = accountToken
        val spaces = mutable.value.spaces
        viewModelScope.launch {
            try {
                val destination = readVoiceDestination(api, token, spaces, spaceId, channelId) ?: return@launch
                if (!context.isCurrent(generation, accountGeneration, VoiceCallService.navigationEpoch(), VoiceCallService.state.value)) return@launch
                val previousSpace = mutable.value.selectedSpace?.space?.id
                val request = ++generation
                if (previousSpace != spaceId) ++spaceAccessGeneration
                closeChannel(clearPending = true)
                installHistoryPins(destination.history)
                mutable.value = mutable.value.copy(
                    selectedSpace = destination.detail, selectedChannel = destination.channel,
                    messages = mergeTimelinePins(destination.history.messages), hasMoreMessages = destination.history.hasMore,
                    presencePage = if (previousSpace == spaceId) mutable.value.presencePage else 0,
                    deniedVoiceChannels = if (previousSpace == spaceId) mutable.value.deniedVoiceChannels else emptySet(),
                    busy = false, error = null,
                )
                renewDraftUploads(destination.channel.id)
                openGateway(channelId, destination.history.cursor, request)
                onOpened()
            } catch (error: Throwable) {
                // A denied or failed destination must not revoke the channel being read.
                if (context.isCurrent(generation, accountGeneration, VoiceCallService.navigationEpoch(), VoiceCallService.state.value) &&
                    error !is ApiException) fail(error)
            }
        }
    }

    /** Web's Try again (failed first load) and Retry (failed refresh). */
    fun retryMessages() {
        val current = mutable.value
        val channel = current.selectedChannel ?: return
        if (current.messagesError != null) {
            if (current.visit != null) lastVisit?.takeIf { it.channelId == channel.id }?.let(::openFeedbackConversation)
            else if (channel.direct) current.directConversations.firstOrNull { it.id == channel.id }?.let(::selectDirect)
            else selectChannel(channel)
        }
        else if (current.refreshError != null || current.hasNewerMessages) resyncChannel(channel.id)
    }

    fun loadOlder() {
        val channel = mutable.value.selectedChannel ?: return
        val before = mutable.value.channelMessages.firstOrNull()?.seq ?: return
        val request = generation
        val anchorRequest = historyAnchorRequest
        if (refreshingHistory || !mutable.value.hasMoreMessages || mutable.value.loadingOlder) return
        mutable.value = mutable.value.copy(loadingOlder = true, olderError = null)
        viewModelScope.launch {
            try {
                val history = api.history(accountToken, channel.id, before)
                if (request != generation || anchorRequest != historyAnchorRequest) return@launch
                val newer = authoritativeMessages()
                mutable.value = mutable.value.copy(
                    messages = projectMessages(mergeTimelinePins(mergeMessages(newer, history.messages, unloadedReactions, unloadedAttachments))), hasMoreMessages = history.hasMore,
                    threadOnlyRows = mutable.value.threadOnlyRows - history.messages.map { it.id }.toSet(),
                    contextStart = if (mutable.value.contextStart != null) history.messages.firstOrNull()?.seq ?: mutable.value.contextStart else null,
                    loadingOlder = false,
                )
            } catch (error: Throwable) {
                if (request == generation && anchorRequest == historyAnchorRequest) mutable.value = mutable.value.copy(loadingOlder = false, olderError = message(error))
            }
        }
    }

    fun openForwardedConversation(wrapper: ChatMessage, readOnly: () -> Unit, onOpened: () -> Unit = {}) {
        if (openingForward) return
        openingForward = true
        val request = generation; val accountRequest = accountGeneration; val navigation = conversationOpeningRevision
        viewModelScope.launch {
            try {
                val token = requireAccountToken()
                val conversation = forwardedConversation(wrapper)
                if (request != generation || accountRequest != accountGeneration || navigation != conversationOpeningRevision) return@launch
                val source = conversation.source
                if (source == null) { readOnly(); return@launch }
                require(source.message.id == conversation.root?.id && source.message.channelId == conversation.root?.channelId) { "Conversation is unavailable." }
                val target = source.message
                if (mutable.value.selectedChannel?.id != target.channelId) {
                    // Prepare before replacing the destination, just as voice-context navigation does.
                    val detail = source.spaceId?.let { api.space(token, it) }
                    val direct = if (detail == null) api.directConversations(token).conversations.first { it.id == target.channelId } else null
                    val channel = detail?.channels?.first { it.id == target.channelId }
                        ?: Channel(target.channelId, "", requireNotNull(direct).peer.displayName, private = true, direct = true)
                    val history = api.history(token, target.channelId)
                    require(history.channel?.id == target.channelId && history.space?.id == (source.spaceId ?: "")) { "Conversation is unavailable." }
                    if (request != generation || accountRequest != accountGeneration || navigation != conversationOpeningRevision) return@launch
                    rememberConversationHistory(); cancelConversationOpening()
                    val previousSpace = mutable.value.selectedSpace?.space?.id
                    val committed = ++generation
                    if (detail != null && previousSpace != detail.space.id) ++spaceAccessGeneration
                    closeChannel(clearPending = true)
                    if (detail != null) lastChannelBySpace[detail.space.id] = channel.id
                    installHistoryPins(history)
                    mutable.value = mutable.value.copy(
                        selectedSpace = detail ?: mutable.value.selectedSpace, selectedChannel = channel, selectedDirectId = direct?.id,
                        directConversations = if (direct == null) mutable.value.directConversations else mutable.value.directConversations.filter { it.id != direct.id } + direct,
                        messages = mergeTimelinePins(history.messages), hasMoreMessages = history.hasMore,
                        presencePage = if (detail == null || previousSpace == detail.space.id) mutable.value.presencePage else 0,
                        deniedVoiceChannels = if (detail == null || previousSpace == detail.space.id) mutable.value.deniedVoiceChannels else emptySet(),
                        busy = false, error = null, messagesLoading = false,
                    )
                    renewDraftUploads(channel.id)
                    openGateway(channel.id, history.cursor, committed, channel.joined && direct?.incoming != true)
                    if (direct != null) refreshPeople()
                }
                goToMessage(target, notify = true) {
                    if (target.threadRootId == null) openThread(target.id)
                    onOpened()
                }
            } catch (error: CancellationException) { throw error }
            catch (error: Throwable) {
                if (request == generation && accountRequest == accountGeneration && navigation == conversationOpeningRevision)
                    mutable.value = mutable.value.copy(error = "Couldn’t open the conversation. Try again.")
            } finally { openingForward = false }
        }
    }

    /** [notify]: also show a failure as the app's notice, for jumps made where the Pins list (and its error) isn't shown. */
    fun goToMessage(target: ChatMessage, notify: Boolean = false, onLoaded: () -> Unit) {
        goToMessage(target.channelId, target.id, target.threadRootId,
            onFailed = { if (notify) mutable.value = mutable.value.copy(error = mutable.value.messageContextError) }, onLoaded = onLoaded)
    }

    /** Loads [targetId]'s context: around it in the conversation, or in its thread [root] (which opens). */
    private fun goToMessage(channelId: String, targetId: String, root: String?, onFailed: () -> Unit = {}, onLoaded: () -> Unit) {
        val channel = mutable.value.selectedChannel?.id ?: return
        if (channelId != channel || mutable.value.loadingMessageContext || refreshingHistory) return
        val request = generation; val anchor = ++historyAnchorRequest
        mutable.value = mutable.value.copy(loadingMessageContext = true, messageContextError = null, loadingOlder = false, loadingNewer = false)
        viewModelScope.launch {
            try {
                val page = if (root == null) api.history(accountToken, channel, around = targetId) else null
                val threadPage = if (root != null) api.thread(accountToken, channel, root, around = targetId) else null
                if (request != generation || anchor != historyAnchorRequest) return@launch
                val context = page?.messages ?: requireNotNull(threadPage).messages
                require(context.any { it.id == targetId }) { "Message context is unavailable." }
                val loaded = mutable.value.messages.map { it.id }.toSet()
                val incoming = threadPage?.let { listOf(it.root) + context } ?: context
                ++threadRequest
                mutable.value = mutable.value.copy(
                    messages = projectMessages(mergeTimelinePins(mergeMessages(authoritativeMessages(), incoming, unloadedReactions))),
                    thread = threadPage?.let { ThreadUi(requireNotNull(root), loading = false, hasMore = it.hasMore, before = context.firstOrNull()?.seq,
                        hasNewer = it.hasNewer, after = context.lastOrNull()?.seq, windowStart = context.firstOrNull()?.seq, windowEnd = if (it.hasNewer) context.lastOrNull()?.seq else null) },
                    threadOnlyRows = if (root == null) mutable.value.threadOnlyRows - context.map { it.id }.toSet() else mutable.value.threadOnlyRows + incoming.filter { it.id !in loaded && (it.threadRootId == null || it.broadcast) }.map { it.id },
                    contextStart = page?.messages?.firstOrNull()?.seq ?: mutable.value.contextStart,
                    contextEnd = if (page != null) if (page.hasNewer) context.lastOrNull()?.seq else null else mutable.value.contextEnd,
                    hasMoreMessages = page?.hasMore ?: mutable.value.hasMoreMessages,
                    hasNewerMessages = page?.hasNewer ?: mutable.value.hasNewerMessages,
                    focusedMessageId = targetId, focusRevision = mutable.value.focusRevision + 1,
                    loadingMessageContext = false, loadingOlder = false, loadingNewer = false,
                )
                mutable.value.thread?.let { threadPages[it.rootId] = it }
                onLoaded()
            } catch (error: Throwable) {
                if (error is CancellationException) throw error
                if (request == generation && anchor == historyAnchorRequest) {
                    mutable.value = mutable.value.copy(loadingMessageContext = false, messageContextError = message(error))
                    onFailed()
                }
            }
        }
    }

    fun loadNewer() {
        val channel = mutable.value.selectedChannel?.id ?: return
        val after = mutable.value.channelMessages.lastOrNull()?.seq ?: return
        if (!mutable.value.hasNewerMessages || mutable.value.loadingNewer || refreshingHistory) return
        val request = generation; val anchor = historyAnchorRequest
        mutable.value = mutable.value.copy(loadingNewer = true, refreshError = null)
        viewModelScope.launch {
            try {
                val page = api.history(accountToken, channel, after = after)
                if (request != generation || anchor != historyAnchorRequest) return@launch
                mutable.value = mutable.value.copy(
                    messages = projectMessages(mergeTimelinePins(mergeMessages(authoritativeMessages(), page.messages, unloadedReactions))),
                    threadOnlyRows = mutable.value.threadOnlyRows - page.messages.map { it.id }.toSet(),
                    contextEnd = if (page.hasNewer) page.messages.lastOrNull()?.seq ?: after else null,
                    hasNewerMessages = page.hasNewer, loadingNewer = false,
                )
            } catch (error: Throwable) {
                if (request == generation && anchor == historyAnchorRequest) mutable.value = mutable.value.copy(loadingNewer = false, refreshError = message(error))
            }
        }
    }

    fun setPresencePage(page: Int) {
        val members = mutable.value.selectedSpace?.members ?: return
        val max = ((members.size - 1).coerceAtLeast(0)) / PRESENCE_PAGE_SIZE
        val next = page.coerceIn(0, max)
        mutable.value = mutable.value.copy(presencePage = next, presence = emptyMap())
        watchVisiblePresence()
    }

    private val availabilityRequests = mutableMapOf<String, Job>()

    /** Fetch `${mediaRoot}/status` independently for each stable channel action. */
    fun checkVoiceAvailability(channel: Channel? = mutable.value.selectedChannel) {
        val current = mutable.value
        channel ?: return
        if (visitedVoiceChannel() == channel.id) return checkVisitedVoice(channel.id)
        val detail = current.selectedSpace ?: return
        if (!channel.joined || detail.channels.none { it.id == channel.id && it.joined }) return
        val spaceId = detail.space.id
        val demo = detail.space.demo
        val key = voiceRootKey(demo, channel.id)
        val token = accountToken
        val accountRequest = accountGeneration
        availabilityRequests[key]?.cancel()
        availabilityRequests[key] = viewModelScope.launch {
            val enabled = withTimeoutOrNull(10_000) {
                runCatching { api.mediaStatus(token, channel.id, demo).enabled }.getOrElse { if (it is kotlinx.coroutines.CancellationException) throw it; false }
            } ?: false
            if (accountRequest == accountGeneration && mutable.value.selectedSpace?.let { it.space.id == spaceId && it.channels.any { item -> item.id == channel.id } } == true)
                mutable.value = mutable.value.copy(voiceAvailability = mutable.value.voiceAvailability + (key to enabled))
        }
    }

    /** A visited space channel, whose voice a team member may join like a member (never a DM). */
    private fun visitedVoiceChannel(): String? = mutable.value.visit?.takeIf { !it.direct && it.spaceId != null }?.channelId

    /** Voice rosters to watch: the open space's joined channels, plus a visited space channel. */
    private fun mediaChannels(detail: SpaceDetail?): List<String> =
        (listOfNotNull(visitedVoiceChannel()) + detail?.channels?.filter { it.joined }?.map { it.id }.orEmpty()).distinct().take(24)

    /** The visited channel's media status, kept while that visit stays open. */
    private fun checkVisitedVoice(channelId: String) {
        val token = accountToken
        val accountRequest = accountGeneration
        availabilityRequests[channelId]?.cancel()
        availabilityRequests[channelId] = viewModelScope.launch {
            val enabled = withTimeoutOrNull(10_000) {
                runCatching { api.mediaStatus(token, channelId, false).enabled }.getOrElse { if (it is kotlinx.coroutines.CancellationException) throw it; false }
            } ?: false
            if (accountRequest == accountGeneration && visitedVoiceChannel() == channelId)
                mutable.value = mutable.value.copy(voiceAvailability = mutable.value.voiceAvailability + (channelId to enabled))
        }
    }

    private val preparedVoice = mutableMapOf<String, Long>()

    /**
     * Web's `media.prepare` when a signed-in member reaches for Join: the API
     * keeps the session 8 s, so reissue at most every 4 s. The public demo
     * creates on join; failures only mean an ordinary join.
     */
    fun prepareVoiceJoin(channel: Channel) {
        val current = mutable.value
        val token = accountToken ?: return
        if (current.selectedSpace?.space?.demo != false || current.voiceAvailable(channel) != true ||
            !channel.joined || current.selectedSpace.channels.none { it.id == channel.id && it.joined } || channel.id in current.deniedVoiceChannels) return
        val now = android.os.SystemClock.elapsedRealtime()
        if (preparedVoice[channel.id]?.let { now - it < 4_000 } == true) return
        preparedVoice[channel.id] = now
        viewModelScope.launch {
            runCatching { api.media<Unit>(token, channel.id, "prepare") }
                .onFailure { if (it is kotlinx.coroutines.CancellationException) throw it }
        }
    }

    fun reportActivity() { gateway?.reportActivity(); profileFeed?.reportActivity() }
    fun localPresence(): String = gateway?.localPresence() ?: "offline"

    /**
     * Opens the profiles feed once signed in and home, and closes it on logout or in the
     * background (the foreground refreshes repair what it missed meanwhile). A new sign-in
     * gets a new socket; events from an earlier one are dropped.
     */
    private fun syncProfileFeed() {
        val token = accountToken
        val wanted = foreground && token != null && mutable.value.account != null && mutable.value.screen == SessionScreen.Home
        if (wanted && profileFeed != null && profileFeedAccount == accountGeneration) return
        profileFeed?.close(); profileFeed = null
        if (!wanted || token == null) return
        val request = accountGeneration
        profileFeedAccount = request
        profileFeed = ProfileFeedClient(api.baseUrl, token,
            onProfile = { event -> viewModelScope.launch { if (request == accountGeneration) receiveProfile(event) } },
        ).also { it.start() }
    }

    /** Web: a start goes out at most every [TYPING_INTERVAL_MS], and that long without typing sends the stop. */
    fun setTyping(active: Boolean) {
        typingIdle?.cancel(); typingIdle = null
        typingActive = active
        if (active) typingIdle = viewModelScope.launch { delay(TYPING_INTERVAL_MS); typingIdle = null; setTyping(false) }
        flushTyping()
    }

    private fun flushTyping() {
        val token = chatToken ?: return
        val connection = gateway ?: return
        val active = typingActive
        if (!active && !typingSent) return
        val now = android.os.SystemClock.elapsedRealtime()
        if (active && typingSent && now - typingSentAt < TYPING_INTERVAL_MS) return
        typingSent = active; typingSentAt = now
        connection.sendTyping(token, active)
    }

    fun setReaction(messageId: String, emoji: String, active: Boolean) {
        val channel = mutable.value.selectedChannel?.takeIf { it.joined } ?: return
        val key = "$messageId:$emoji"
        val target = (mutable.value.messages + mutable.value.pinnedMessages).firstOrNull { it.id == messageId } ?: return
        val own = mutable.value.chatAuthorId ?: mutable.value.account?.id ?: return
        authoritativeReactionMessages.putIfAbsent(messageId, target)
        reactionIntents.getOrPut(messageId) { linkedMapOf() }[emoji] = ReactionIntent(emoji, active, ++reactionIntentVersion)
        mutable.value = mutable.value.copy(reactionSaves = mutable.value.reactionSaves + (key to ReactionSaveUi(emoji, active)))
        projectPendingReaction(messageId, own)
        if (reactionWorkers[messageId]?.isActive == true) return
        val request = generation
        val requestAccountGeneration = accountGeneration
        val token = accountToken
        reactionWorkers[messageId] = viewModelScope.launch {
            while (true) {
                val intent = reactionIntents[messageId]?.values?.firstOrNull() ?: break
                try {
                    val capability = chatToken ?: createChatSession(requestAccountGeneration) ?: throw IllegalStateException("Chat session is unavailable.")
                    if (requestAccountGeneration != accountGeneration || token != accountToken || request != generation ||
                        mutable.value.selectedChannel?.let { it.id == channel.id && it.joined } != true) break
                    val update = api.setReaction(token, capability, channel.id, messageId, intent.emoji, intent.active)
                    if (request != generation || mutable.value.selectedChannel?.id != channel.id) break
                    receiveReaction(update, sequenced = false)
                    val current = reactionIntents[messageId]?.get(intent.emoji)
                    if (current?.version == intent.version) {
                        reactionIntents[messageId]?.remove(intent.emoji)
                        mutable.value = mutable.value.copy(reactionSaves = mutable.value.reactionSaves - "$messageId:${intent.emoji}")
                    }
                    projectPendingReaction(messageId, own)
                } catch (error: Throwable) {
                    if (error is kotlinx.coroutines.CancellationException) throw error
                    if (request == generation) {
                        if (lostChannel(error)) revokeChannel()
                        else {
                            val current = reactionIntents[messageId]?.get(intent.emoji)
                            if (current?.version == intent.version) {
                                reactionIntents[messageId]?.remove(intent.emoji)
                                mutable.value = mutable.value.copy(reactionSaves = mutable.value.reactionSaves +
                                    ("$messageId:${intent.emoji}" to ReactionSaveUi(intent.emoji, intent.active, false, message(error))))
                                projectPendingReaction(messageId, own)
                            }
                        }
                    }
                }
            }
            if (request == generation) {
                reactionIntents[messageId]?.takeIf { it.isEmpty() }?.let { reactionIntents.remove(messageId) }
                reactionWorkers.remove(messageId)
            }
        }
    }

    fun setPin(messageId: String, active: Boolean) {
        val channel = mutable.value.selectedChannel?.takeIf { it.joined } ?: return
        if (pinWorkers[messageId]?.isActive == true) return
        val target = (mutable.value.messages + mutable.value.pinnedMessages).firstOrNull { it.id == messageId } ?: return
        val own = chatAuthor ?: mutable.value.account?.let {
            ChatAuthor(it.id, it.displayName ?: it.username ?: "You", false, it.avatarId, avatarAsset = it.avatarAsset, avatarUrl = it.avatarUrl)
        } ?: return
        val request = generation
        mutable.value = mutable.value.copy(
            pinSaves = mutable.value.pinSaves + (messageId to PinSaveUi(active)),
            pinIntents = mutable.value.pinIntents + (messageId to PinIntentUi(target, if (active) MessagePin(own, Instant.now().toString()) else null)),
        )
        pinWorkers[messageId] = viewModelScope.launch {
            try {
                val capability = chatToken ?: createChatSession(accountGeneration) ?: error("Chat session is unavailable.")
                if (request != generation || mutable.value.selectedChannel?.id != channel.id) return@launch
                val update = api.setPin(accountToken, capability, channel.id, messageId, active)
                if (request == generation && mutable.value.selectedChannel?.id == channel.id) {
                    receivePin(update, sequenced = false)
                    mutable.value = mutable.value.copy(pinSaves = mutable.value.pinSaves - messageId)
                }
            } catch (error: Throwable) {
                if (error is kotlinx.coroutines.CancellationException) throw error
                if (request == generation) {
                    if (lostChannel(error)) revokeChannel()
                    else mutable.value = mutable.value.copy(pinSaves = mutable.value.pinSaves + (messageId to PinSaveUi(active, false, message(error))))
                }
            } finally {
                if (request == generation) {
                    pinWorkers.remove(messageId)
                    mutable.value = mutable.value.copy(pinIntents = mutable.value.pinIntents - messageId)
                }
            }
        }
    }

    fun retryPin(messageId: String) { mutable.value.pinSaves[messageId]?.let { setPin(messageId, it.active) } }
    fun dismissPinError(messageId: String) { mutable.value = mutable.value.copy(pinSaves = mutable.value.pinSaves - messageId) }

    suspend fun forwardDestinations(): List<ForwardDestination> = api.forwardDestinations(requireNotNull(accountToken) { "Sign in required." }).destinations

    suspend fun forward(source: ChatMessage, destination: String, key: java.util.UUID, text: String): ChatMessage {
        val epoch = accountGeneration
        val token = requireNotNull(accountToken) { "Sign in required." }
        val capability = chatToken ?: createChatSession(epoch) ?: error("Chat session is unavailable.")
        require(epoch == accountGeneration) { "Account changed." }
        val message = api.forward(token, capability, source, destination, key, text)
        if (epoch == accountGeneration && mutable.value.selectedChannel?.id == destination) addMessage(message)
        return message
    }

    suspend fun forwardedConversation(source: ChatMessage, before: String? = null): ForwardConversation =
        api.forwardedConversation(requireNotNull(accountToken) { "Sign in required." }, source, before)

    private fun authoritativeMessages(): List<ChatMessage> = mutable.value.messages.map {
        authoritativeReactionMessages[it.id] ?: it
    }

    private fun projectMessages(messages: List<ChatMessage>): List<ChatMessage> {
        val own = mutable.value.chatAuthorId ?: mutable.value.account?.id
        return messages.map { message ->
            if (message.id in authoritativeReactionMessages) authoritativeReactionMessages[message.id] = message
            val intents = reactionIntents[message.id]?.values.orEmpty().map { ReactionSaveUi(it.emoji, it.active) }
            if (own == null) message else projectReactionIntents(message, own, intents)
        }
    }

    private fun projectPendingReaction(messageId: String, own: String) {
        val authoritative = authoritativeReactionMessages[messageId] ?: return
        val intents = reactionIntents[messageId]?.values.orEmpty().map { ReactionSaveUi(it.emoji, it.active) }
        fun project(message: ChatMessage): ChatMessage = if (message.id == messageId)
            projectReactionIntents(message.copy(reactions = authoritative.reactions, reactionSeq = authoritative.reactionSeq), own, intents)
        else message
        mutable.value = mutable.value.copy(
            messages = mutable.value.messages.map(::project),
            pinnedMessages = mutable.value.pinnedMessages.map(::project),
        )
    }

    fun retryReaction(messageId: String, emoji: String) {
        val save = mutable.value.reactionSaves["$messageId:$emoji"] ?: return
        setReaction(messageId, emoji, save.active)
    }

    fun dismissReactionError(messageId: String, emoji: String) {
        val key = "$messageId:$emoji"
        mutable.value = mutable.value.copy(reactionSaves = mutable.value.reactionSaves - key)
    }

    /** Who reacted to [message] in the open conversation, reused until its reaction revision changes. */
    suspend fun reactors(message: ChatMessage): ReactorList {
        reactorCache.get(message.id, message.reactionSeq)?.let { return it }
        val channel = mutable.value.selectedChannel?.takeIf { it.id == message.channelId }
            ?: throw IllegalStateException("This conversation is no longer open.")
        val requestAccountGeneration = accountGeneration
        val list = api.reactors(accountToken, channel.id, message.id)
        check(requestAccountGeneration == accountGeneration && mutable.value.selectedChannel?.id == channel.id) {
            "This conversation is no longer open."
        }
        reactorCache.put(list)
        return list
    }

    internal fun authorizeVoiceJoin(intent: VoiceJoinIntent, onAuthorized: () -> Unit, onFailure: (String) -> Unit) {
        val request = ++voiceAuthorizationRequest
        val spaceRequest = spaceAccessGeneration
        val token = accountToken
        viewModelScope.launch {
            try {
                // The cached sidebar listing is not an authorization decision. Check fresh
                // server access before start() synchronously replaces a healthy current call.
                // A visited channel (its space is not yours) is checked through its own media status.
                val accessible = if (!intent.demo && visitedVoiceChannel() == intent.channelId && mutable.value.visit?.spaceId == intent.spaceId) {
                    if (api.mediaStatus(checkNotNull(token), intent.channelId, false).enabled) setOf(intent.channelId) else emptySet()
                } else api.space(checkNotNull(token), intent.spaceId).let { detail ->
                    if (detail.space.id != intent.spaceId) emptySet() else detail.channels.filter { it.joined }.mapTo(mutableSetOf()) { it.id }
                }
                if (request != voiceAuthorizationRequest || spaceRequest != spaceAccessGeneration ||
                    !intent.isCurrent(mutable.value, accountGeneration) ||
                    !VoiceCallService.joinAuthorizationCurrent(intent.controlEpoch)) return@launch
                if (intent.isCurrent(mutable.value, accountGeneration, accessible)) onAuthorized()
                else {
                    mutable.value = mutable.value.copy(
                        deniedVoiceChannels = mutable.value.deniedVoiceChannels + intent.channelId,
                        voiceRosters = mutable.value.voiceRosters - intent.channelId,
                        voiceSessionStartedAt = mutable.value.voiceSessionStartedAt - intent.channelId,
                    )
                    onFailure("Voice is not available in this channel.")
                }
            } catch (error: Throwable) {
                if (request == voiceAuthorizationRequest && spaceRequest == spaceAccessGeneration &&
                    intent.isCurrent(mutable.value, accountGeneration) &&
                    VoiceCallService.joinAuthorizationCurrent(intent.controlEpoch)) {
                    onFailure(message(error))
                }
            }
        }
    }

    fun closeThread() { uploadSend?.cancel(); ++threadRequest; mutable.value = mutable.value.copy(thread = null) }

    fun openThread(root: String) {
        if (mutable.value.thread?.rootId == root) return
        uploadSend?.cancel()
        ++threadRequest
        val cached = threadPages[root]
        mutable.value = mutable.value.copy(thread = cached ?: ThreadUi(root))
        mutable.value.selectedChannel?.id?.let { renewDraftUploads(it) }
        if (cached == null) loadThread()
    }

    fun loadThread(older: Boolean = false, newer: Boolean = false) {
        val thread = mutable.value.thread ?: return
        val channel = mutable.value.selectedChannel?.id ?: return
        val request = ++threadRequest
        val channelRequest = generation
        mutable.value = mutable.value.copy(thread = thread.copy(loading = true, error = null))
        viewModelScope.launch {
            try {
                val page = api.thread(accountToken, channel, thread.rootId, if (older) thread.before else null, after = if (newer) thread.after else null)
                if (request != threadRequest || channelRequest != generation || mutable.value.thread?.rootId != thread.rootId) return@launch
                val rows = listOf(page.root) + page.messages
                val loaded = mutable.value.messages.map { it.id }.toSet()
                val pageState = thread.copy(loading = false, error = null, failedOlder = false, failedNewer = false,
                    hasMore = if (newer) thread.hasMore else page.hasMore,
                    before = if (newer) thread.before else page.messages.firstOrNull()?.seq ?: if (older) thread.before else null,
                    hasNewer = if (older) thread.hasNewer else page.hasNewer,
                    after = if (older) thread.after else page.messages.lastOrNull()?.seq ?: if (newer) thread.after else null,
                    windowStart = if (newer) thread.windowStart else page.messages.firstOrNull()?.seq ?: if (older) thread.windowStart else null,
                    windowEnd = if (older) thread.windowEnd else if (page.hasNewer) page.messages.lastOrNull()?.seq else null)
                threadPages[thread.rootId] = pageState
                mutable.value = mutable.value.copy(
                    messages = projectMessages(mergeTimelinePins(mergeMessages(authoritativeMessages(), rows, unloadedReactions, unloadedAttachments))),
                    threadOnlyRows = mutable.value.threadOnlyRows + rows.filter { it.id !in loaded && (it.threadRootId == null || it.broadcast) }.map { it.id },
                    thread = pageState,
                )
            } catch (error: Throwable) {
                if (request != threadRequest || channelRequest != generation) return@launch
                if (error is ApiException && error.status in listOf(401, 403, 404)) { closeThread(); resyncChannel(channel) }
                else mutable.value = mutable.value.copy(thread = thread.copy(loading = false, error = message(error), failedOlder = older, failedNewer = newer))
            }
        }
    }

    fun replyTo(message: ChatMessage, inThread: Boolean = false) {
        val state = mutable.value
        if (!state.canParticipate || state.pendingMessage != null || message.channelId != state.selectedChannel?.id) return
        val root = if (inThread) state.thread?.rootId ?: return else if (message.threadRootId != null && !message.broadcast) message.threadRootId else null
        val key = root ?: "channel"
        val text = message.content.text.ifEmpty { if (message.forward != null) "Forwarded message" else "" }
        val excerpt = text.substring(0, text.offsetByCodePoints(0, minOf(200, text.codePointCount(0, text.length))))
        val reference = MessageReply(message.id, message.author, excerpt, message.content.attachments.isNotEmpty(), message.threadRootId,
            fullText = text.takeIf { it.codePointCount(0, it.length) > 200 })
        mutable.value = state.copy(replyTargets = state.replyTargets + (key to reference))
        if (!inThread && root != null) openThread(root)
    }

    fun cancelReply(root: String = "channel") { mutable.value = mutable.value.copy(replyTargets = mutable.value.replyTargets - root) }

    private var uploadSend: Job? = null

    /** Preflight leaves the whole selection in the composer until every upload is ready. */
    fun send(text: String, confirmed: () -> Unit = {}, threadRootId: String? = null, broadcast: Boolean = false,
        unchanged: () -> Boolean = { true }, accepted: () -> Unit = {},
    ): Boolean {
        if (uploadSend?.isActive == true) return false
        val channel = mutable.value.selectedChannel?.takeIf { it.joined } ?: return false
        val target = AttachmentDraftTarget(channel.id, threadRootId)
        val files = mutable.value.attachmentDrafts[target].orEmpty()
        if (mutable.value.pendingMessage == null && files.any { it.attachment == null }) {
            val request = generation
            val account = accountGeneration
            val thread = threadRequest
            val reply = mutable.value.replyTargets[threadRootId ?: "channel"]?.id
            val keys = files.map { it.key }
            uploadSend = viewModelScope.launch {
                files.filter { it.attachment == null && uploadJobs[it.key]?.isActive != true }.forEach { retryDraft(it.key) }
                keys.mapNotNull { uploadJobs[it] }.forEach { it.join() }
                if (request != generation || account != accountGeneration) return@launch
                if (chatAuthor == null && createChatSession(account) == null) return@launch
                val current = mutable.value
                if (request != generation || account != accountGeneration || current.selectedChannel?.id != channel.id ||
                    (threadRootId != null && (thread != threadRequest || current.thread?.rootId != threadRootId)) || !unchanged() ||
                    current.replyTargets[threadRootId ?: "channel"]?.id != reply ||
                    current.pendingMessage != null || !current.canParticipate) return@launch
                val ready = current.attachmentDrafts[target].orEmpty()
                if (ready.map { it.key } != keys || ready.any { it.attachment == null || it.error != null }) return@launch
                if (sendReady(text, confirmed, threadRootId, broadcast)) accepted()
            }
            return false
        }
        return sendReady(text, confirmed, threadRootId, broadcast)
    }

    /** Returns false when nothing was sent, so the composer keeps its text and files. */
    private fun sendReady(text: String, confirmed: () -> Unit, threadRootId: String?, broadcast: Boolean): Boolean {
        if (mutable.value.pendingMessage?.let { it.error == null || it.rejected } == true) return false
        if (mutable.value.pendingMessage?.let { it.threadRootId != threadRootId } == true) return false
        val channel = mutable.value.selectedChannel?.takeIf { it.joined } ?: return false
        val target = AttachmentDraftTarget(channel.id, threadRootId)
        val drafts = mutable.value.attachmentDrafts[target].orEmpty()
        val author = chatAuthor ?: run {
            if (text.isBlank() && drafts.isEmpty()) return false
            // The session is still being created (or failed earlier): create
            // it now and send once it exists, instead of refusing the message.
            val request = generation
            val accountRequest = accountGeneration
            viewModelScope.launch {
                val session = createChatSession(accountRequest)
                if (request != generation || accountRequest != accountGeneration || mutable.value.selectedChannel?.id != channel.id) return@launch
                if (session != null && chatAuthor != null) send(text, confirmed, threadRootId, broadcast)
                else fail(IllegalStateException("Chat session is unavailable."))
            }
            return true
        }
        // An unknown outcome must be retried with the same ID, text and files.
        val retry = pendingSends.retrying(channel.id, author)
        val files = if (retry != null) mutable.value.pendingMessage?.attachments.orEmpty() else {
            if (drafts.any { it.attachment == null }) {
                return false
            }
            // Pending messages show the local compressed copies until the server's message arrives.
            drafts.mapNotNull { draft -> draft.attachment?.copy(status = "ready", url = draft.localUrl, previewUrl = draft.thumbnail, spoiler = draft.spoiler) }
        }
        if (retry == null && text.isBlank() && files.isEmpty()) return false
        val request = generation
        var operation = pendingSends.begin(channel.id, author, text, files.map { it.id }, threadRootId, broadcast, mutable.value.replyTargets[threadRootId ?: "channel"], files.filter { it.spoiler }.map { it.id }, confirmed)
        if (retry == null) {
            pendingDrafts = drafts
            pendingNeedsUpload = drafts.any { uploadStale(it) }
        }
        if (retry == null) {
            // If a server ever reports these as processing, this device shows its own picked images
            // (the working copies are deleted once the send is confirmed; the picked URI is not).
            val local = drafts.mapNotNull { draft -> draft.attachment?.id?.let { id -> draft.pickedImage?.let { id to it } } }
            setAttachmentDrafts(target, emptyList())
            mutable.value = mutable.value.copy(
                localAttachmentPreviews = (mutable.value.localAttachmentPreviews + local).entries.toList().takeLast(64).associate { it.key to it.value },
            )
        }
        mutable.value = mutable.value.copy(replyTargets = mutable.value.replyTargets - (threadRootId ?: "channel"), pendingMessage = PendingMessageUi(
            operation.id.toString(), operation.text, author, Instant.now().toString(), attachments = files,
            threadRootId = operation.threadRootId, broadcast = operation.broadcast, replyTo = operation.replyTo,
        ))
        viewModelScope.launch {
            try {
                val capability = chatToken ?: createChatSession(accountGeneration) ?: return@launch
                var recovered = false
                var sent: ChatMessage
                while (true) {
                    if (pendingNeedsUpload) {
                        val token = accountToken ?: throw CancellationException()
                        val refreshed = pendingDrafts.map { uploadDraft(it.key, token, channel.id) }
                        if (request != generation) return@launch
                        operation = pendingSends.replaceAttachments(operation.id, refreshed.map { it.id }) ?: return@launch
                        pendingNeedsUpload = false
                        mutable.value = mutable.value.copy(pendingMessage = mutable.value.pendingMessage?.copy(
                            clientMessageId = operation.id.toString(), attachments = refreshed.mapIndexed { index, attachment ->
                                val draft = pendingDrafts[index]
                                attachment.copy(url = draft.localUrl, previewUrl = draft.thumbnail, status = "ready", spoiler = draft.spoiler)
                            }, error = null, uploadFailed = false,
                        ))
                    }
                    try {
                        sent = retryUnknownSend {
                            api.sendMessage(accountToken, capability, channel.id, author, operation.id, operation.text,
                                operation.attachmentIds, operation.threadRootId, operation.broadcast, operation.replyTo?.id, operation.spoilerAttachmentIds)
                        }
                        break
                    } catch (error: ApiException) {
                        if (error.code != "attachment_unavailable" || error.status != 409) throw error
                        if (request != generation || mutable.value.pendingMessage?.clientMessageId != operation.id.toString()) return@launch
                        pendingNeedsUpload = true
                        pendingDrafts = pendingDrafts.map { it.copy(uploadedAt = 0) }
                        if (recovered) throw UploadException("Failed to upload.")
                        recovered = true
                    }
                }
                if (request == generation && mutable.value.selectedChannel?.id == channel.id) {
                    addMessage(sent)
                    confirmPending(sent)
                    // A reply may answer a report: the inbox shows it on return.
                    if (mutable.value.visit != null) refreshFeedback()
                }
            } catch (error: Throwable) {
                if (request != generation || mutable.value.pendingMessage?.clientMessageId != operation.id.toString()) return@launch
                if (pendingNeedsUpload) {
                    mutable.value = mutable.value.copy(pendingMessage = mutable.value.pendingMessage?.copy(
                        error = "Failed to upload.", uploadFailed = true, rejected = false,
                    ))
                    return@launch
                }
                // A DM privacy or block refusal keeps the conversation; it is not a lost channel.
                if (error is ApiException && directMessageError(error.code) != null) {
                    pendingSends.definitiveFailure(operation.id)
                    mutable.value = mutable.value.copy(pendingMessage = mutable.value.pendingMessage?.copy(error = message(error), rejected = true))
                    if (error.code == DM_BLOCKED) { refreshBlocks(); refreshDirectConversations() }
                    return@launch
                }
                if (error is ApiException && error.status == 401) chatToken = null
                if (error is ApiException) when (classifySendFailure(error.status)) {
                    SendFailure.REVOKED -> { pendingSends.definitiveFailure(operation.id); revokeChannel() }
                    SendFailure.DEFINITIVE -> {
                        pendingSends.definitiveFailure(operation.id)
                        mutable.value = mutable.value.copy(pendingMessage = mutable.value.pendingMessage?.copy(error = message(error), rejected = true))
                    }
                    SendFailure.UNKNOWN -> mutable.value = mutable.value.copy(pendingMessage = mutable.value.pendingMessage?.copy(error = message(error)))
                } else mutable.value = mutable.value.copy(pendingMessage = mutable.value.pendingMessage?.copy(error = message(error)))
            }
        }
        return true
    }

    /** Edit restores a rejected message's files to the composer; Dismiss drops them. */
    fun discardPending(restoreFiles: Boolean = false, restoreReply: Boolean = restoreFiles): String? {
        val pending = mutable.value.pendingMessage ?: return null
        if (!pending.rejected) return null
        val channel = mutable.value.selectedChannel?.id ?: return null
        val target = AttachmentDraftTarget(channel, pending.threadRootId)
        val drafts = mutable.value.attachmentDrafts[target].orEmpty()
        pendingSends.clear()
        val candidates = if (restoreFiles) pendingDrafts + drafts else drafts
        val kept = candidates.take(AttachmentPolicy.MAX_ATTACHMENTS)
        (pendingDrafts + drafts).filter { it !in kept }.forEach { abandonDraft(it) }
        pendingDrafts = emptyList()
        pendingNeedsUpload = false
        val targets = if (restoreReply && pending.replyTo != null) mutable.value.replyTargets + ((pending.threadRootId ?: "channel") to pending.replyTo) else mutable.value.replyTargets
        setAttachmentDrafts(target, kept)
        mutable.value = mutable.value.copy(pendingMessage = null, replyTargets = targets)
        return pending.text
    }

    // --- Attachments ---------------------------------------------------------------------------

    private fun checkUploads(request: Long) {
        val token = accountToken ?: return
        viewModelScope.launch {
            val usage = try { api.assetUsage(token) } catch (cancelled: CancellationException) { throw cancelled } catch (_: Throwable) { null }
            if (request != accountGeneration) return@launch
            usage?.let { compression = it.compression }
            mutable.value = mutable.value.copy(uploadsEnabled = usage != null, uploadsChecked = true)
        }
    }

    /** Picked photos, videos or documents: compress on the device, upload, and hold as composer drafts. */
    fun addAttachments(uris: List<Uri>, target: AttachmentDraftTarget, epoch: Long = accountGeneration) {
        if (epoch != accountGeneration) return
        addAttachments(uris, null, target)
    }

    internal fun addProfileAttachments(person: String, uris: List<Uri>) = addAttachments(uris, person)

    /** Clipboard providers grant URI access; copy the bytes locally just like picked files. */
    internal fun pasteProfileAttachments(person: String) {
        val clipboard = getApplication<Application>().getSystemService(ClipboardManager::class.java)
        val clip = clipboard.primaryClip
        val uris = (0 until (clip?.itemCount ?: 0)).mapNotNull { clip?.getItemAt(it)?.uri }
            .filter { it.scheme == "content" }.distinct()
        if (uris.isEmpty()) profileDrafts.error(person, "No files on the clipboard. Copy a file or use Attach files.")
        else addProfileAttachments(person, uris)
    }

    private fun addAttachments(uris: List<Uri>, person: String?, target: AttachmentDraftTarget? = null) {
        val current = mutable.value
        if ((person == null && target == null) || accountToken == null) return
        if (person == null && current.selectedChannel?.id == target?.channelId && !current.canParticipate) return
        if (!current.uploadsEnabled || uris.isEmpty()) return
        if (person != null && !profileDrafts[person].editable) return
        val files = if (person == null) current.attachmentDrafts[target].orEmpty() else profileDrafts[person].files
        val room = AttachmentPolicy.MAX_ATTACHMENTS - files.size
        if (room <= 0) {
            if (person == null) setAttachmentDrafts(requireNotNull(target), files, "You can attach up to ${AttachmentPolicy.MAX_ATTACHMENTS} files.")
            else profileDrafts.error(person, "You can attach up to ${AttachmentPolicy.MAX_ATTACHMENTS} files.")
            return
        }
        val error = if (uris.size > room) "Only $room more file${if (room == 1) "" else "s"} can be attached." else null
        val settings = compression
        val request = accountGeneration
        uris.take(room).forEach { uri ->
            val key = UUID.randomUUID().toString()
            val directory = File(uploadSession, key)
            draftDirectories[key] = directory
            val draft = DraftAttachmentUi(key, "File", "file", pickedUri = uri.toString())
            if (person == null) {
                setAttachmentDrafts(requireNotNull(target), mutable.value.attachmentDrafts[target].orEmpty() + draft, error)
                retryDraft(key)
                return@forEach
            }
            // Profile files stay local until Send supplies a real DM destination.
            profileDrafts.add(person, listOf(draft))
            uploadJobs[key] = viewModelScope.launch {
                try {
                    val picked = preparer.copy(uri, directory)
                    val image = picked.contentType.startsWith("image/")
                    updateDraft(key, person) {
                        it.copy(
                            name = picked.name, kind = AttachmentPolicy.kind(picked.contentType), sourceSize = picked.source.length(),
                            thumbnail = if (image) Uri.fromFile(picked.source).toString() else null, pickedImage = if (image) uri.toString() else null,
                        )
                    }
                    val prepared = preparer.prepare(picked, settings, directory, progressReporter(key, person))
                    if (request != accountGeneration) return@launch
                    profileDrafts.ready(person, key, prepared)
                    updateDraft(key, person) {
                        it.copy(
                            name = prepared.name, kind = prepared.kind, storedSize = prepared.file.length(), compressing = false, progress = 0f,
                            thumbnail = (prepared.preview ?: prepared.file.takeIf { prepared.kind == "image" })?.let { file -> Uri.fromFile(file).toString() } ?: it.thumbnail,
                            localUrl = Uri.fromFile(prepared.file).toString(),
                        )
                    }
                } catch (cancelled: CancellationException) {
                    throw cancelled
                } catch (error: Throwable) {
                    updateDraft(key, person) { it.copy(error = AttachmentPolicy.uploadErrorMessage(error), compressing = false) }
                } finally {
                    uploadJobs.remove(key)
                }
            }
        }
        if (person != null && uris.size > room) profileDrafts.error(person, "Only $room more file${if (room == 1) "" else "s"} can be attached.")
    }

    private fun renewDraftUploads(channelId: String) {
        // Renew anything that failed or expired while the conversation was closed.
        val files = mutable.value.attachmentDrafts.filterKeys { it.channelId == channelId }.values.flatten()
        files.filter { it.error != null || uploadStale(it) }.forEach { retryDraft(it.key) }
    }

    private fun setAttachmentDrafts(target: AttachmentDraftTarget, files: List<DraftAttachmentUi>, error: String? = null) {
        mutable.value = mutable.value.copy(
            attachmentDrafts = if (files.isEmpty()) mutable.value.attachmentDrafts - target else mutable.value.attachmentDrafts + (target to files),
            attachmentErrors = if (error == null) mutable.value.attachmentErrors - target else mutable.value.attachmentErrors + (target to error),
        )
    }

    /** Gives up a conversation's regular and thread files when it can no longer be used. */
    private fun dropChannelDrafts(channelId: String) {
        mutable.value.attachmentDrafts.filterKeys { it.channelId == channelId }.values.flatten().forEach { abandonDraft(it) }
        mutable.value = mutable.value.copy(
            attachmentDrafts = mutable.value.attachmentDrafts.filterKeys { it.channelId != channelId },
            attachmentErrors = mutable.value.attachmentErrors.filterKeys { it.channelId != channelId },
        )
    }

    /** Open, sending and stashed drafts: an upload continues while its file is in any of them. */
    private fun knownDrafts() = mutable.value.attachmentDrafts.values.flatten() + pendingDrafts

    private fun uploadStale(draft: DraftAttachmentUi) = draft.uploadedAt?.let { System.currentTimeMillis() - it >= 23 * 60 * 60 * 1000L } == true

    fun retryDraft(key: String) {
        val target = mutable.value.attachmentDrafts.entries.find { it.value.any { draft -> draft.key == key } }?.key ?: return
        val token = accountToken ?: return
        if (uploadJobs[key]?.isActive == true) return
        uploadJobs[key] = viewModelScope.launch {
            try { uploadDraft(key, token, target.channelId) }
            catch (cancelled: CancellationException) { throw cancelled }
            catch (_: Throwable) { updateDraft(key) { it.copy(error = "Failed to upload.", compressing = false) } }
            finally { uploadJobs.remove(key) }
        }
    }

    private suspend fun uploadDraft(key: String, token: String, channel: String): ChatAttachment {
        val draft = knownDrafts().find { it.key == key } ?: throw CancellationException()
        if (draft.attachment != null && !uploadStale(draft)) return draft.attachment
        updateDraft(key) { it.copy(attachment = null, error = null, progress = 0f) }
        draft.attachment?.let { api.cancelAsset(token, it.id) }
        val prepared = preparedDrafts[key] ?: run {
            val uri = Uri.parse(draft.pickedUri ?: throw UploadException("Failed to upload."))
            val directory = draftDirectories[key] ?: throw CancellationException()
            val picked = preparer.copy(uri, directory)
            val image = picked.contentType.startsWith("image/")
            updateDraft(key) { it.copy(name = picked.name, kind = AttachmentPolicy.kind(picked.contentType),
                sourceSize = picked.source.length(), thumbnail = if (image) Uri.fromFile(picked.source).toString() else null,
                pickedImage = if (image) uri.toString() else null) }
            preparer.prepare(picked, compression, directory, progressReporter(key)).also {
                if (knownDrafts().none { it.key == key }) throw CancellationException()
                preparedDrafts[key] = it
            }
        }
        updateDraft(key) { it.copy(name = prepared.name, kind = prepared.kind, storedSize = prepared.file.length(),
            compressing = false, progress = 0f, localUrl = Uri.fromFile(prepared.file).toString(),
            thumbnail = (prepared.preview ?: prepared.file.takeIf { prepared.kind == "image" })?.let { file -> Uri.fromFile(file).toString() } ?: it.thumbnail) }
        val attachment = uploader.upload(token, channel, prepared, progressReporter(key))
        if (knownDrafts().none { it.key == key }) {
            api.cancelAsset(token, attachment.id)
            throw CancellationException()
        }
        updateDraft(key) { it.copy(attachment = attachment, progress = 1f, uploadedAt = System.currentTimeMillis(), error = null) }
        return attachment
    }

    fun toggleDraftSpoiler(key: String) {
        updateDraft(key) { it.copy(spoiler = !it.spoiler) }
    }

    internal fun toggleProfileAttachmentSpoiler(person: String, key: String) {
        profileDrafts.updateFile(person, key) { it.copy(spoiler = !it.spoiler) }
    }

    fun removeDraft(key: String) {
        val entry = mutable.value.attachmentDrafts.entries.find { it.value.any { draft -> draft.key == key } } ?: return
        entry.value.find { it.key == key }?.let { abandonDraft(it) }
        setAttachmentDrafts(entry.key, entry.value.filter { it.key != key })
    }

    private fun abandonDraft(draft: DraftAttachmentUi) {
        val token = accountToken
        val id = draft.attachment?.id
        if (token != null && id != null) viewModelScope.launch(NonCancellable) { api.cancelAsset(token, id) }
        discardDraftFiles(draft.key)
    }

    internal fun removeProfileAttachment(person: String, key: String) {
        if (profileDrafts.remove(person, key)) discardDraftFiles(key)
    }

    /** Stops an unfinished upload and deletes its working copies; the API expires reservations never sent. */
    private fun discardDraftFiles(key: String) {
        uploadJobs.remove(key)?.cancel()
        preparedDrafts.remove(key)
        draftDirectories.remove(key)?.let { directory -> viewModelScope.launch(Dispatchers.IO) { directory.deleteRecursively() } }
    }

    private fun updateDraft(key: String, person: String? = null, change: (DraftAttachmentUi) -> DraftAttachmentUi) {
        if (person != null) { profileDrafts.updateFile(person, key, change); return }
        mutable.value = mutable.value.copy(attachmentDrafts = mutable.value.attachmentDrafts.mapValues { (_, files) ->
            files.map { if (it.key == key) change(it) else it }
        })
        pendingDrafts = pendingDrafts.map { if (it.key == key) change(it) else it }
    }

    /** Compression/upload progress arrives off the main thread; post at most every 1%. */
    private fun progressReporter(key: String, person: String? = null): (Float) -> Unit {
        var last = -1f
        return { fraction ->
            if (fraction - last >= 0.01f || fraction >= 1f) {
                last = fraction
                viewModelScope.launch(Dispatchers.Main) { updateDraft(key, person) { if (it.attachment == null) it.copy(progress = fraction) else it } }
            }
        }
    }

    /** A 403/404 (or expired) media load asks for new signatures once per file. */
    fun reportAttachmentFailure(attachment: ChatAttachment, status: Int?) {
        if (urlRefresh.afterLoadFailure(attachment, status)) requestAttachmentUrls(listOf(attachment.id))
    }

    private fun refreshExpiringUrls() {
        val current = mutable.value
        val attachments = current.messages.flatMap { it.content.attachments }.map { it.withFreshUrls(current.freshAttachmentUrls[it.id]) }
        requestAttachmentUrls(urlRefresh.expiring(attachments))
    }

    private fun requestAttachmentUrls(ids: List<String>) {
        val valid = ids.filter { runCatching { it.assetPathId() }.isSuccess }
        if (valid.isEmpty() || accountToken == null) return
        urlQueue += valid
        if (urlBatch?.isActive == true) return
        val request = generation
        urlBatch = viewModelScope.launch {
            delay(250)
            while (urlQueue.isNotEmpty() && request == generation) {
                val batch = urlQueue.take(100)
                urlQueue.removeAll(batch.toSet())
                val token = accountToken ?: break
                val urls = try { api.attachmentUrls(token, batch) } catch (cancelled: CancellationException) { throw cancelled } catch (_: Throwable) { continue }
                if (request != generation) break
                if (urls.isNotEmpty()) mutable.value = mutable.value.copy(freshAttachmentUrls = mutable.value.freshAttachmentUrls + urls)
            }
        }
    }

    private suspend fun retryUnknownSend(block: suspend () -> ChatMessage): ChatMessage = try { block() } catch (error: IOException) {
        if (error is ApiException) throw error
        delay(250)
        block()
    }

    private suspend fun createChatSession(requestAccountGeneration: Long): String? {
        if (chatToken != null) return chatToken
        return try {
            val name = mutable.value.account?.displayName ?: "Guest"
            val session = api.chatSession(accountToken, name)
            if (requestAccountGeneration != accountGeneration) return null
            val account = mutable.value.account
            if (account != null) require(session.author.id == account.id && !session.author.isGuest) { "Chat identity mismatch." }
            chatToken = session.token
            chatAuthor = session.author
            mutable.value = mutable.value.copy(sessionError = null, chatAuthorId = session.author.id)
            session.token
        } catch (error: Throwable) {
            if (requestAccountGeneration == accountGeneration) mutable.value = mutable.value.copy(sessionError = message(error))
            null
        }
    }

    /** Web's Retry session. */
    fun retrySession() { viewModelScope.launch { createChatSession(accountGeneration) } }

    private fun openGateway(channel: String, cursor: String, request: Long, participating: Boolean = true) {
        durableReplayCursor = cursor
        markConversationRead(channel, cursor)
        val connection = GatewayClient(
            baseUrl = api.baseUrl, token = accountToken, channelId = channel, initialCursor = cursor,
            // Profile and space changes come from the account-wide [profileFeed], not per conversation.
            onMessage = { value -> viewModelScope.launch {
                if (generation == request) {
                    durableReplayCursor = value.seq
                    addMessage(value)
                }
            } },
            onReaction = { value -> viewModelScope.launch { if (generation == request) receiveReaction(value) } },
            onAttachments = { value -> viewModelScope.launch { if (generation == request) receiveAttachments(value) } },
            onAttachmentProgress = { value -> viewModelScope.launch { if (generation == request) receiveAttachmentProgress(value) } },
            onPin = { value -> viewModelScope.launch { if (generation == request) receivePin(value) } },
            onForward = { value -> viewModelScope.launch { if (generation == request) receiveForward(value) } },
            onEdit = { value -> viewModelScope.launch { if (generation == request) receiveEdit(value) } },
            onTyping = { author, active, revision -> viewModelScope.launch { if (participating && generation == request) receiveTyping(author, active, revision) } },
            onPresence = { snapshot -> viewModelScope.launch {
                if (participating && generation == request) mutable.value = mutable.value.copy(presence = snapshot.members.associate { it.userId to it.status })
            } },
            onMedia = { channelId, people, sessionStartedAt -> viewModelScope.launch {
                if (participating && generation == request && (channelId.isEmpty() && mutable.value.selectedSpace?.space?.demo == true ||
                    (mutable.value.selectedSpace?.channels?.any { it.id == channelId } == true || visitedVoiceChannel() == channelId) &&
                    channelId !in mutable.value.deniedVoiceChannels)) {
                    mutable.value = mutable.value.copy(
                        voiceRosters = mutable.value.voiceRosters + (channelId to people),
                        voiceSessionStartedAt = if (sessionStartedAt == null) mutable.value.voiceSessionStartedAt - channelId
                            else mutable.value.voiceSessionStartedAt + (channelId to sessionStartedAt),
                    )
                }
            } },
            onMediaDenied = { channelId -> viewModelScope.launch {
                if (generation == request) {
                    VoiceCallService.stopIfChannel(getApplication(), channelId)
                    mutable.value = mutable.value.copy(
                        voiceRosters = mutable.value.voiceRosters - channelId,
                        voiceSessionStartedAt = mutable.value.voiceSessionStartedAt - channelId,
                        deniedVoiceChannels = mutable.value.deniedVoiceChannels + channelId,
                    )
                }
            } },
            onMediaDisconnected = { viewModelScope.launch {
                if (generation == request) mutable.value = mutable.value.copy(voiceRosters = emptyMap(), voiceSessionStartedAt = emptyMap())
            } },
            onAccessDenied = { viewModelScope.launch { if (generation == request) revokeChannel() } },
            onResync = { viewModelScope.launch { if (generation == request) resyncChannel(channel) } },
        )
        gateway = connection
        gatewayStatus = viewModelScope.launch {
            connection.status.collect { status -> if (generation == request) mutable.value = mutable.value.copy(gateway = status) }
        }
        connection.start()
        if (participating) watchVisiblePresence()
        val detail = mutable.value.selectedSpace
        if (participating && (detail != null || mutable.value.visit != null)) connection.watchMedia(mediaChannels(detail), detail?.space?.demo == true)
    }

    private fun watchVisiblePresence() {
        val detail = mutable.value.selectedSpace ?: return
        if (detail.space.demo || accountToken == null || detail.members.isEmpty()) return
        val start = mutable.value.presencePage * PRESENCE_PAGE_SIZE
        // A private channel's readers lead the member list's pages.
        val ids = detail.memberOrder(mutable.value.selectedChannel?.id).drop(start).take(PRESENCE_PAGE_SIZE).map { it.id }
        if (ids.isNotEmpty()) gateway?.watchPresence(detail.space.id, ids)
    }

    private fun receiveTyping(author: ChatAuthor, active: Boolean, revision: String) {
        if (author.id == chatAuthor?.id) return
        val next = revision.toLongOrNull() ?: return
        val old = typers[author.id]
        if (old != null && next <= old.revision) return
        if (old == null && typers.size >= 64) return
        // Keep stop revisions until expiry so delayed starts cannot revive typing.
        typers[author.id] = TypingAuthor(author, next, System.currentTimeMillis() + 6_000, active)
        refreshTypers()
    }

    private fun refreshTypers() {
        val now = System.currentTimeMillis()
        typers.entries.removeAll { it.value.expiresAt <= now }
        val blocked = mutable.value.blockedIds
        mutable.value = mutable.value.copy(typingAuthors = typers.values.filter { it.typing && it.author.id != chatAuthor?.id && it.author.id !in blocked }.map { it.author })
        typingExpiry?.cancel()
        val next = typers.values.minOfOrNull { it.expiresAt } ?: return
        typingExpiry = viewModelScope.launch { delay((next - now).coerceAtLeast(1)); refreshTypers() }
    }

    private fun addMessage(message: ChatMessage) {
        if (message.channelId != mutable.value.selectedChannel?.id) return
        if (message.threadRootId != null) refreshThreadUnread()
        val messages = authoritativeMessages()
        val isNew = messages.none { it.id == message.id }
        mutable.value = mutable.value.copy(messages = projectMessages(mergeTimelinePins(mergeMessages(messages, listOf(message), unloadedReactions, unloadedAttachments))))
        if (isNew) {
            // Web chimes for someone else's new message in the open conversation.
            // Never for a blocked author or an incoming request.
            val current = mutable.value
            if (message.author.id != chatAuthor?.id && !collapsesFor(message, current.blockedIds, chatAuthor?.id) && current.selectedDirect?.incoming != true)
                chat.caper.android.ui.CaperEffects.play(chat.caper.android.ui.CaperEffects.Effect.Message)
        }
        confirmPending(message)
        markConversationRead(message.channelId, message.seq)
    }

    private fun receiveReaction(update: ReactionUpdate, sequenced: Boolean = true) {
        if (update.channelId != mutable.value.selectedChannel?.id) return
        durableReplayCursor = replayCursorAfterReaction(durableReplayCursor, update, sequenced)
        if (sequenced) markConversationRead(update.channelId, update.seq)
        pinSnapshots[update.messageId]?.let { pinSnapshots[update.messageId] = mergeReaction(it, update) }
        val current = authoritativeReactionMessages[update.messageId]
            ?: (mutable.value.messages + mutable.value.pinnedMessages).firstOrNull { it.id == update.messageId }
        if (current == null) {
            if (!cacheUnseenReaction(unloadedReactions, update)) {
                mutable.value.selectedChannel?.id?.let(::resyncChannel)
                return
            }
            return
        }
        val authoritative = mergeReaction(current, update)
        authoritativeReactionMessages[update.messageId] = authoritative
        mutable.value = mutable.value.copy(
            messages = mutable.value.messages.map { mergeReaction(it, update) },
            pinnedMessages = mutable.value.pinnedMessages.map { mergeReaction(it, update) },
        )
        val own = mutable.value.chatAuthorId ?: mutable.value.account?.id
        if (own != null) projectPendingReaction(update.messageId, own)
    }

    /** `message.attachments`: sequenced exactly like reactions, replacing the message's files. */
    private fun receiveAttachments(update: AttachmentsUpdate) {
        if (update.channelId != mutable.value.selectedChannel?.id) return
        durableReplayCursor = update.seq
        markConversationRead(update.channelId, update.seq)
        // Pinned snapshots can hold messages outside the loaded page; keep their files current too.
        pinSnapshots[update.messageId]?.let { pinSnapshots[update.messageId] = mergeAttachments(it, update) }
        val pinned = mutable.value.pinnedMessages.map { mergeAttachments(it, update) }
        if (mutable.value.messages.none { it.id == update.messageId }) {
            mutable.value = mutable.value.copy(pinnedMessages = pinned)
            if (!cacheUnseenAttachments(unloadedAttachments, update)) mutable.value.selectedChannel?.id?.let(::resyncChannel)
            return
        }
        authoritativeReactionMessages[update.messageId]?.let { authoritativeReactionMessages[update.messageId] = mergeAttachments(it, update) }
        val settled = update.attachments.filter { it.state != AttachmentState.PROCESSING }.map { it.id }.toSet()
        mutable.value = mutable.value.copy(
            messages = mutable.value.messages.map { mergeAttachments(it, update) },
            pinnedMessages = pinned,
            attachmentProgress = mutable.value.attachmentProgress - settled,
        )
    }

    /** Ephemeral processing percent, like typing: latest value wins and nothing is replayed. */
    private fun receiveAttachmentProgress(progress: AttachmentProgress) {
        val current = mutable.value
        val processing = current.messages.firstOrNull { it.id == progress.messageId }?.content?.attachments
            ?.any { it.id == progress.attachmentId && it.state == AttachmentState.PROCESSING } == true
        if (!processing || current.attachmentProgress[progress.attachmentId] == progress.percent) return
        if (progress.attachmentId !in current.attachmentProgress && current.attachmentProgress.size >= 256) return
        mutable.value = current.copy(attachmentProgress = current.attachmentProgress + (progress.attachmentId to progress.percent))
    }

    private fun receiveEdit(update: EditUpdate) {
        if (update.channelId != mutable.value.selectedChannel?.id) return
        durableReplayCursor = update.seq
        markConversationRead(update.channelId, update.seq)
        applyEditSnapshot(update.message)
    }

    // HTTP confirmations update content only; they are not delivery/read cursors.
    private fun applyEditSnapshot(message: ChatMessage) {
        val loadedIds = (mutable.value.messages + mutable.value.pinnedMessages).map { it.id }.toSet()
        if (!cacheEditSnapshot(editSnapshots, message, loadedIds)) {
            resyncChannel(message.channelId)
            return
        }
        authoritativeReactionMessages[message.id]?.let { authoritativeReactionMessages[message.id] = mergeEdit(it, message) }
        pinSnapshots[message.id]?.let { pinSnapshots[message.id] = mergeEdit(it, message) }
        mutable.value = mutable.value.copy(
            messages = mutable.value.messages.map { mergeEdit(it, message) },
            pinnedMessages = mutable.value.pinnedMessages.map { mergeEdit(it, message) },
        )
    }

    fun canEdit(message: ChatMessage): Boolean = message.forward == null && mutable.value.canParticipate &&
        mutable.value.selectedChannel?.id == message.channelId &&
        chatAuthor?.let { !it.isGuest && it.id == message.author.id } == true

    suspend fun editMessage(message: ChatMessage, text: String): Unit {
        check(canEdit(message)) { "Only the author can edit while participating." }
        check(message.id !in mutable.value.editIntents) { "This message is already being saved." }
        val request = generation
        val authorId = requireNotNull(chatAuthor).id
        mutable.value = mutable.value.copy(editIntents = mutable.value.editIntents + (message.id to EditIntentUi(text, message.revision)))
        try {
            val capability = chatToken ?: createChatSession(accountGeneration) ?: error("Chat session is unavailable.")
            val result = try {
                api.editMessage(accountToken, capability, message.channelId, message.id, text, message.revision)
            } catch (error: ApiException) {
                if (error.status == 401 && chatToken == capability) chatToken = null
                throw error
            }
            if (request != generation || !canEdit(message)) throw kotlinx.coroutines.CancellationException()
            require(result.author.id == authorId) { "Message author mismatch." }
            applyEditSnapshot(result)
        } finally {
            if (request == generation) mutable.value = mutable.value.copy(editIntents = mutable.value.editIntents - message.id)
        }
    }

    suspend fun reloadMessage(message: ChatMessage): ChatMessage {
        val request = generation
        val result = api.loadMessage(accountToken, message.channelId, message.id)
        if (request != generation || mutable.value.selectedChannel?.id != message.channelId) throw kotlinx.coroutines.CancellationException()
        applyEditSnapshot(result)
        return result
    }

    suspend fun messageVersions(message: ChatMessage, before: Int? = null): MessageVersions {
        val request = generation
        val result = api.messageVersions(accountToken, message.channelId, message.id, before)
        if (request != generation || mutable.value.selectedChannel?.id != message.channelId) throw kotlinx.coroutines.CancellationException()
        return result
    }

    private fun receivePin(update: PinUpdate, sequenced: Boolean = true) {
        if (update.channelId != mutable.value.selectedChannel?.id) return
        durableReplayCursor = replayCursorAfterPin(durableReplayCursor, update, sequenced)
        if (sequenced) markConversationRead(update.channelId, update.seq)
        update.message.reactionSeq?.let { seq ->
            receiveReaction(ReactionUpdate("message.reactions", 1, update.channelId, seq, update.message.id, update.message.reactions), sequenced = false)
        }
        if (pinSnapshotCursor?.let { update.seq.toBigInteger() <= it.toBigInteger() } == true) return
        val old = pinSnapshots[update.message.id]
        val candidate = overlayReactions(overlayEdit(update.message))
        val merged = if (old == null) candidate else mergeReaction(mergeEdit(mergePin(old, candidate), candidate), candidate)
        pinSnapshots[update.message.id] = merged
        authoritativeReactionMessages[merged.id]?.let { authoritativeReactionMessages[merged.id] = mergeReaction(mergeEdit(mergePin(it, merged), merged), merged) }
        mutable.value = mutable.value.copy(
            messages = mutable.value.messages.map { if (it.id == merged.id) mergeReaction(mergeEdit(mergePin(it, merged), merged), merged) else it },
            pinnedMessages = pinSnapshots.values.filter { it.pin != null }.sortedByDescending { it.pinSeq?.toBigIntegerOrNull() },
        )
        val own = mutable.value.chatAuthorId ?: mutable.value.account?.id
        if (own != null) projectPendingReaction(merged.id, own)
    }

    private fun receiveForward(update: ForwardUpdate) {
        if (update.channelId != mutable.value.selectedChannel?.id) return
        durableReplayCursor = update.seq
        markConversationRead(update.channelId, update.seq)
        val previous = forwardSnapshots[update.message.id] ?: mutable.value.messages.firstOrNull { it.id == update.message.id }
        val snapshot = previous?.let { mergeForward(it, update.message) } ?: update.message
        forwardSnapshots[update.message.id] = snapshot
        if (forwardSnapshots.size > 256) forwardSnapshots.remove(forwardSnapshots.keys.first())
        authoritativeReactionMessages[snapshot.id]?.let { authoritativeReactionMessages[snapshot.id] = mergeForward(it, snapshot) }
        pinSnapshots[snapshot.id]?.let { pinSnapshots[snapshot.id] = mergeForward(it, snapshot) }
        mutable.value = mutable.value.copy(
            messages = mutable.value.messages.map { mergeForward(it, snapshot) },
            pinnedMessages = mutable.value.pinnedMessages.map { mergeForward(it, snapshot) },
        )
    }

    private fun installHistoryPins(history: ChatHistory) {
        val listed = history.pinnedMessages.associateBy { it.id }
        val cursor = history.cursor.toBigIntegerOrNull()
        pinSnapshotCursor = history.cursor
        if (cursor != null) (mutable.value.messages + pinSnapshots.values.toList()).forEach { message ->
            val current = pinSnapshots[message.id]?.let { mergePin(message, it) } ?: message
            if (current.id !in listed && (current.pinSeq?.toBigIntegerOrNull() ?: java.math.BigInteger.valueOf(-1)) <= cursor) {
                // Retain a tombstone: a delayed acknowledgement/page must not
                // restore a pin removed while this client was disconnected.
                pinSnapshots[current.id] = current.copy(pin = null, pinSeq = history.cursor)
            }
        }
        listed.forEach { (id, message) ->
            val candidate = overlayReactions(overlayEdit(message))
            pinSnapshots[id] = pinSnapshots[id]?.let { mergeReaction(mergeEdit(mergePin(it, candidate), candidate), candidate) } ?: candidate
        }
        mutable.value = mutable.value.copy(pinnedMessages = pinSnapshots.values.filter { it.pin != null }.sortedByDescending { it.pinSeq?.toBigIntegerOrNull() })
    }

    private fun overlayReactions(message: ChatMessage): ChatMessage {
        var current = message
        val loaded = authoritativeReactionMessages[message.id] ?: mutable.value.messages.find { it.id == message.id }
        for (snapshot in listOfNotNull(loaded, pinSnapshots[message.id])) current = mergeReaction(current, snapshot)
        return unloadedReactions[message.id]?.let { mergeReaction(current, it) } ?: current
    }

    private fun overlayEdit(message: ChatMessage): ChatMessage {
        val loaded = mutable.value.messages.find { it.id == message.id }
        val current = loaded?.let { mergeEdit(message, it) } ?: message
        return editSnapshots[message.id]?.let { mergeEdit(current, it) } ?: current
    }

    private fun mergeTimelinePins(messages: List<ChatMessage>) = messages.map { message ->
        val pinned = overlayReactions(overlayEdit(overlayPin(message, pinSnapshots[message.id], pinSnapshotCursor)))
        forwardSnapshots[message.id]?.let { mergeForward(pinned, it) } ?: pinned
    }

    private fun confirmPending(message: ChatMessage) {
        pendingSends.confirm(message)?.let {
            mutable.value = mutable.value.copy(pendingMessage = null, focusedMessageId = null)
            pendingDrafts.forEach { draft -> discardDraftFiles(draft.key) }
            pendingDrafts = emptyList()
            pendingNeedsUpload = false
            it.confirmed.invoke()
            if (!refreshingHistory) {
                if (message.threadRootId != null && mutable.value.thread?.rootId == message.threadRootId && mutable.value.thread?.hasNewer == true) loadThread()
                else if (message.threadRootId == null && mutable.value.hasNewerMessages) resyncChannel(message.channelId)
            }
        }
    }

    private fun resyncChannel(channelId: String) {
        val channel = mutable.value.selectedChannel?.takeIf { it.id == channelId } ?: return
        val previousCursor = durableReplayCursor ?: return
        val request = ++generation
        unloadedReactions.clear()
        unloadedAttachments.clear()
        val previous = mutable.value.copy(messages = authoritativeMessages())
        closeChannel(clearPending = false)
        // closeChannel normally discards channel replay state; a failed refresh must remain retryable.
        durableReplayCursor = previousCursor
        refreshingHistory = true
        // Keep the conversation readable while it reloads, as the web does.
        mutable.value = mutable.value.copy(
            selectedChannel = channel, selectedDirectId = previous.selectedDirectId, visit = previous.visit,
            channelVisitUnread = previous.channelVisitUnread,
            messages = previous.messages, hasMoreMessages = previous.hasMoreMessages,
            thread = previous.thread, threadOnlyRows = previous.threadOnlyRows,
            contextStart = previous.contextStart, contextEnd = previous.contextEnd,
            hasNewerMessages = previous.hasNewerMessages,
            gateway = GatewayStatus.CONNECTING, busy = true, error = null,
        )
        viewModelScope.launch {
            try {
                val history = api.history(accountToken, channel.id)
                if (generation != request || mutable.value.selectedChannel?.id != channel.id) return@launch
                history.messages.forEach(::confirmPending)
                val recovered = recoverHistory(if (previous.hasNewerMessages) emptyList() else authoritativeMessages().filter { (it.threadRootId == null || it.broadcast) && it.id !in previous.threadOnlyRows }, previous.hasMoreMessages, previousCursor, history)
                installHistoryPins(history)
                mutable.value = mutable.value.copy(
                    messages = projectMessages(mergeTimelinePins(recovered.messages)), hasMoreMessages = recovered.hasMore, busy = false,
                    threadOnlyRows = emptySet(),
                    contextStart = null, contextEnd = null, hasNewerMessages = false,
                )
                openGateway(channel.id, history.cursor, request, channel.joined)
                if (mutable.value.thread != null) loadThread()
            } catch (error: Throwable) {
                if (generation == request) {
                    if (error is ApiException && error.status in listOf(401, 403, 404)) revokeChannel()
                    else mutable.value = mutable.value.copy(busy = false, refreshError = message(error))
                }
            } finally {
                if (generation == request) refreshingHistory = false
            }
        }
    }

    fun createSpace(name: String, done: () -> Unit = {}) = launchAction { request ->
        val space = api.createSpace(requireAccountToken(), name)
        if (request != accountGeneration) return@launchAction
        mutable.value = mutable.value.copy(spaces = mutable.value.spaces + space)
        done(); selectSpace(space.id)
    }
    fun renameSpace(name: String, done: () -> Unit = {}) = launchAction { request ->
        val detail = requireNotNull(mutable.value.selectedSpace)
        val context = AdminMutationContext(request, detail.space.id)
        val space = api.updateSpace(requireAccountToken(), detail.space.id, name)
        if (!context.isCurrent(accountGeneration, mutable.value.selectedSpace)) return@launchAction
        replaceDetail(detail.copy(space = space)); done()
    }

    // --- Profile pictures and space icons -------------------------------------------------------

    /**
     * A photo picked for your profile picture ([spaceId] null) or the icon of a space you own:
     * prepared on the device (GIFs unchanged, anything else a WebP square), uploaded, and applied
     * by the server. Runs on with its dialog closed; the state shows progress and the error.
     */
    fun changePicture(uri: Uri, spaceId: String? = null, epoch: Long = accountGeneration) {
        if (epoch != accountGeneration) return
        val key = spaceId ?: PROFILE_PICTURE_KEY
        if (mutable.value.pictureSaves[key]?.busy == true) return
        val token = accountToken ?: return
        val request = accountGeneration
        val target = if (spaceId == null) PictureTarget.Avatar else PictureTarget.SpaceIcon(spaceId)
        setPictureSave(key, PictureSaveUi(uploading = true))
        pictureJobs[key] = viewModelScope.launch {
            val directory = File(uploadSession, "picture-${UUID.randomUUID()}")
            try {
                val picked = preparer.copy(uri, directory)
                val picture = PicturePreparer.prepare(picked.source)
                val result = pictureUploader.upload(token, target, picture)
                if (request != accountGeneration) return@launch
                result.account?.takeIf { spaceId == null }?.let(::applyOwnAccount)
                result.space?.takeIf { spaceId != null }?.let(::applySpace)
                setPictureSave(key, null)
            } catch (error: CancellationException) {
                throw error
            } catch (error: Throwable) {
                if (request == accountGeneration) setPictureSave(key, PictureSaveUi(error = pictureError(error, target)))
            } finally {
                viewModelScope.launch(Dispatchers.IO) { directory.deleteRecursively() }
            }
        }
    }

    /** Back to your saved mascot ([spaceId] null) or the space's initial. */
    fun removePicture(spaceId: String? = null) {
        val key = spaceId ?: PROFILE_PICTURE_KEY
        if (mutable.value.pictureSaves[key]?.busy == true) return
        val token = accountToken ?: return
        val request = accountGeneration
        val target = if (spaceId == null) PictureTarget.Avatar else PictureTarget.SpaceIcon(spaceId)
        setPictureSave(key, PictureSaveUi(removing = true))
        pictureJobs[key] = viewModelScope.launch {
            try {
                if (spaceId == null) {
                    val account = api.removeAvatar(token)
                    if (request != accountGeneration) return@launch
                    applyOwnAccount(account)
                } else {
                    val space = api.removeSpaceIcon(token, spaceId)
                    if (request != accountGeneration) return@launch
                    applySpace(space)
                }
                setPictureSave(key, null)
            } catch (error: CancellationException) {
                throw error
            } catch (error: Throwable) {
                if (request == accountGeneration) setPictureSave(key, PictureSaveUi(error = pictureError(error, target)))
            }
        }
    }

    /** Clears a finished picture change's error ([key]: [PROFILE_PICTURE_KEY] or a space ID); one in flight stays. */
    fun dismissPictureError(key: String) {
        if (mutable.value.pictureSaves[key]?.busy == false) setPictureSave(key, null)
    }

    private fun setPictureSave(key: String, value: PictureSaveUi?) {
        val saves = mutable.value.pictureSaves
        mutable.value = mutable.value.copy(pictureSaves = if (value == null) saves - key else saves + (key to value))
    }

    /**
     * The server's account after a picture change. [state] resolves it into every retained
     * snapshot of you (members, authors, DMs, pins, threads, replies) through the same own-profile
     * refresh as a profile save, so the picture changes everywhere without a reload.
     */
    private fun applyOwnAccount(saved: Account) {
        val current = mutable.value.account ?: return
        if (saved.id != current.id) return
        // Only `GET /api/account/me` reports the feedback team; a picture change keeps it.
        mutable.value = mutable.value.copy(account = saved.copy(feedbackTeam = saved.feedbackTeam || current.feedbackTeam))
    }

    /** A space whose icon changed: its rail tile and, when it is open, the open space (Manage space, header). */
    private fun applySpace(changed: Space) {
        // A background reconcile that started before this change must not put the old icon back.
        membershipRevision++
        mutable.value = mutable.value.withSpace(changed)
    }

    /** [person]'s name and picture in every retained copy: the state and this view model's caches. */
    private fun applyPerson(person: ProfilePerson, keepSignedUrls: Boolean = false) {
        val current = mutable.value.withPerson(person, keepSignedUrls)
        mutable.value = current.copy(profileRevision = current.profileRevision + 1)
        conversationHistories.replaceAll { _, history -> history.withPerson(person, keepSignedUrls) }
        reactorCache.update { it.withPerson(person) }
        typers.replaceAll { _, typing -> typing.copy(author = typing.author.withPerson(person, keepSignedUrls)) }
        listOf(authoritativeReactionMessages, pinSnapshots, forwardSnapshots, editSnapshots).forEach { snapshots ->
            snapshots.replaceAll { _, message -> message.withPerson(person, keepSignedUrls) }
        }
        chatAuthor = chatAuthor?.withPerson(person, keepSignedUrls)
    }

    /**
     * [fetched]: current profiles from a server list requested after [since] `profile.updated`
     * events, already installed. The `profiles` feed has no replay, so a copy still showing an
     * older name or picture (a loaded or cached message, another list) takes the fetched one, as
     * on web and Apple. Someone whose event arrived while the request was in flight keeps that
     * event, also where the list just put the older profile back.
     */
    private fun repairProfiles(fetched: List<ProfilePerson>, since: Long) {
        val newer = fetched.mapNotNull { person -> profileEvents[person.id]?.takeIf { it.first > since }?.second }
        val newerIds = newer.mapTo(HashSet()) { it.id }
        val stale = mutable.value.staleProfiles(fetched.filter { it.id !in newerIds }, conversationHistories.values)
        (newer + stale).forEach { applyPerson(it, keepSignedUrls = true) }
    }

    /**
     * A live `profiles` event. A person's new name or picture reaches every retained copy by
     * account ID, including this view model's caches (cached conversations, who reacted, typing,
     * message snapshots kept for reverts); your own account changed on another device runs the
     * own-profile refresh. A space you're in takes its new name and icon; others are ignored.
     */
    private fun receiveProfile(event: ProfileEvent) {
        when (event) {
            is ProfileEvent.Person -> {
                val person = event.person
                val before = mutable.value.account
                profileEvents[person.id] = ++profileEventCount to person
                applyPerson(person)
                // Renamed on another device: like a profile save here, the chat session takes the new name.
                if (before != null && person.id == before.id && person.displayName != null && person.displayName != before.displayName) {
                    chatToken = null
                    chatAuthor = null
                    mutable.value = mutable.value.copy(chatAuthorId = null)
                    viewModelScope.launch { createChatSession(accountGeneration) }
                }
            }
            is ProfileEvent.SpaceChanged -> {
                val current = mutable.value
                val id = event.space.id
                if (current.spaces.none { it.id == id } && current.invitations.none { it.id == id } && current.selectedSpace?.space?.id != id) return
                applySpace(event.space)
            }
        }
    }
    fun deleteCurrentSpace(done: () -> Unit = {}) = launchAction { request ->
        val detail = requireNotNull(mutable.value.selectedSpace)
        val context = AdminMutationContext(request, detail.space.id)
        api.deleteSpace(requireAccountToken(), detail.space.id)
        if (!context.isCurrent(accountGeneration, mutable.value.selectedSpace)) return@launchAction
        conversationHistories.entries.removeAll { it.value.space?.id == detail.space.id }
        VoiceCallService.stopIfSpace(getApplication(), detail.space.id)
        val remaining = mutable.value.spaces.filter { it.id != detail.space.id }
        mutable.value = mutable.value.copy(spaces = remaining)
        done(); remaining.firstOrNull()?.let { selectSpace(it.id) }
    }
    fun leaveCurrentSpace(done: () -> Unit = {}) = launchAction { request ->
        val detail = requireNotNull(mutable.value.selectedSpace)
        val context = AdminMutationContext(request, detail.space.id)
        val account = requireNotNull(mutable.value.account)
        api.removeSpaceMember(requireAccountToken(), detail.space.id, account.id)
        if (!context.isCurrent(accountGeneration, mutable.value.selectedSpace)) return@launchAction
        conversationHistories.entries.removeAll { it.value.space?.id == detail.space.id }
        VoiceCallService.stopIfSpace(getApplication(), detail.space.id)
        val remaining = mutable.value.spaces.filter { it.id != detail.space.id }
        mutable.value = mutable.value.copy(spaces = remaining)
        done(); remaining.firstOrNull()?.let { selectSpace(it.id) }
    }
    fun createChannel(name: String, privateChannel: Boolean, done: (Channel) -> Unit = {}) = launchAction { request ->
        val detail = requireNotNull(mutable.value.selectedSpace)
        val context = AdminMutationContext(request, detail.space.id)
        val channel = api.createChannel(requireAccountToken(), detail.space.id, name, privateChannel)
        if (!context.isCurrent(accountGeneration, mutable.value.selectedSpace)) return@launchAction
        replaceDetail(detail.copy(channels = detail.channels + channel)); selectChannel(channel); done(channel)
    }
    fun updateChannel(channel: Channel, name: String, privateChannel: Boolean, done: () -> Unit = {}) = launchAction { request ->
        val detail = requireNotNull(mutable.value.selectedSpace)
        val context = AdminMutationContext(request, detail.space.id, channel.id)
        val updated = api.updateChannel(requireAccountToken(), detail.space.id, channel.id, name, privateChannel)
        if (!context.isCurrent(accountGeneration, mutable.value.selectedSpace)) return@launchAction
        replaceDetail(detail.copy(channels = detail.channels.map { if (it.id == channel.id) updated else it }))
        mutable.value = mutable.value.copy(selectedChannel = if (mutable.value.selectedChannel?.id == channel.id) updated else mutable.value.selectedChannel)
        done()
    }
    fun deleteChannel(channel: Channel, done: () -> Unit = {}) = launchAction { request ->
        val detail = requireNotNull(mutable.value.selectedSpace)
        val context = AdminMutationContext(request, detail.space.id, channel.id)
        api.deleteChannel(requireAccountToken(), detail.space.id, channel.id)
        if (!context.isCurrent(accountGeneration, mutable.value.selectedSpace)) return@launchAction
        VoiceCallService.stopIfChannel(getApplication(), channel.id)
        val channels = detail.channels.filter { it.id != channel.id }
        replaceDetail(detail.copy(channels = channels)); done()
        channels.firstOrNull { it.joined }?.let(::selectChannel) ?: closeChannel(clearPending = true)
    }
    fun addSpaceMember(username: String, done: () -> Unit = {}, failed: (String) -> Unit = {}) = launchAction { request ->
        val detail = requireNotNull(mutable.value.selectedSpace)
        val context = AdminMutationContext(request, detail.space.id)
        val member = memberRequest(request, failed) { api.addSpaceMember(requireAccountToken(), detail.space.id, username) } ?: return@launchAction
        if (!context.isCurrent(accountGeneration, mutable.value.selectedSpace)) return@launchAction
        mutable.value = mutable.value.copy(pendingSpaceInvitations = mutable.value.pendingSpaceInvitations.filter { it.id != member.id } + member)
        done()
    }
    /**
     * A profile invitation targets an owned space without changing the open conversation.
     * [already] gets "Member" or "Invited" when the server says it is redundant, before [failed].
     */
    fun inviteToSpace(spaceId: String, username: String, done: () -> Unit, already: (String) -> Unit, failed: (String) -> Unit) = accountRequest(failed) { token, request ->
        val current = mutable.value
        check(current.spaces.any { it.id == spaceId && it.ownerId == current.account?.id && !it.demo }) { "Only space owners can invite people." }
        val member = try { api.addSpaceMember(token, spaceId, username) }
        catch (error: ApiException) {
            redundantInvitations[error.message]?.let { if (request == accountGeneration) already(it) }
            throw error
        }
        if (request != accountGeneration) return@accountRequest
        if (mutable.value.selectedSpace?.space?.id == spaceId)
            mutable.value = mutable.value.copy(pendingSpaceInvitations = mutable.value.pendingSpaceInvitations.filter { it.id != member.id } + member)
        done()
    }
    fun loadSpaceInvitations() = launchAction { request ->
        val detail = requireNotNull(mutable.value.selectedSpace)
        val context = AdminMutationContext(request, detail.space.id)
        val invitations = api.spaceInvitations(requireAccountToken(), detail.space.id).members
        if (context.isCurrent(accountGeneration, mutable.value.selectedSpace))
            mutable.value = mutable.value.copy(pendingSpaceInvitations = invitations)
    }
    fun cancelSpaceInvitation(member: Member) = launchAction { request ->
        val detail = requireNotNull(mutable.value.selectedSpace)
        val context = AdminMutationContext(request, detail.space.id)
        api.cancelSpaceInvitation(requireAccountToken(), detail.space.id, member.id)
        if (context.isCurrent(accountGeneration, mutable.value.selectedSpace))
            mutable.value = mutable.value.copy(pendingSpaceInvitations = mutable.value.pendingSpaceInvitations.filter { it.id != member.id })
    }
    /** The open space's live invite links, for its owner; a failed load is [failed]'s, beside the links. */
    fun loadInviteLinks(loaded: () -> Unit, failed: (String) -> Unit) {
        val detail = mutable.value.selectedSpace ?: return
        val context = AdminMutationContext(accountGeneration, detail.space.id)
        mutable.value = mutable.value.copy(inviteLinks = emptyList())
        viewModelScope.launch {
            try {
                val links = api.inviteLinks(requireAccountToken(), detail.space.id).links
                if (context.isCurrent(accountGeneration, mutable.value.selectedSpace)) { mutable.value = mutable.value.copy(inviteLinks = links); loaded() }
            } catch (error: CancellationException) { throw error }
            catch (error: Throwable) { if (context.isCurrent(accountGeneration, mutable.value.selectedSpace)) failed(message(error)) }
        }
    }
    /** Creates a link for the open space; [done] gets it (now first in the list) to copy. */
    fun createInviteLink(channelId: String?, maxAge: Int?, maxUses: Int?, done: (InviteLink) -> Unit, failed: (String) -> Unit) = launchAction { request ->
        val detail = requireNotNull(mutable.value.selectedSpace)
        val context = AdminMutationContext(request, detail.space.id)
        val link = formRequest(request, failed) { api.createInviteLink(requireAccountToken(), detail.space.id, channelId, maxAge, maxUses) } ?: return@launchAction
        if (!context.isCurrent(accountGeneration, mutable.value.selectedSpace)) return@launchAction
        mutable.value = mutable.value.copy(inviteLinks = listOf(link) + mutable.value.inviteLinks.filter { it.code != link.code })
        done(link)
    }
    /** A link that is already gone (404: revoked elsewhere, expired or used up) leaves the list too. */
    fun revokeInviteLink(link: InviteLink, failed: (String) -> Unit) = launchAction { request ->
        val detail = requireNotNull(mutable.value.selectedSpace)
        val context = AdminMutationContext(request, detail.space.id)
        formRequest(request, failed) {
            try { api.revokeInviteLink(requireAccountToken(), detail.space.id, link.code) }
            catch (error: ApiException) { if (error.status != 404) throw error }
            Unit
        } ?: return@launchAction
        if (context.isCurrent(accountGeneration, mutable.value.selectedSpace))
            mutable.value = mutable.value.copy(inviteLinks = mutable.value.inviteLinks.filter { it.code != link.code })
    }
    fun inviteUrl(code: String): String = api.inviteUrl(code)
    /** What a live invite link shares before joining; an invalid or expired one fails with [INVALID_INVITE_LINK]. */
    fun previewInvite(code: String, done: (InvitePreview) -> Unit, failed: (String) -> Unit) {
        val request = accountGeneration
        viewModelScope.launch {
            try {
                val preview = api.invitePreview(code, accountToken)
                if (request == accountGeneration) done(preview)
            } catch (error: CancellationException) { throw error }
            catch (error: Throwable) { if (request == accountGeneration) failed(inviteError(error)) }
        }
    }
    /**
     * Joins with an invite link (or, already a member, just opens the space), refreshes the spaces
     * list and opens the space at the link's channel, else its default.
     */
    fun joinInvite(code: String, done: () -> Unit, failed: (String) -> Unit) = launchAction { request ->
        val token = requireAccountToken()
        val result = try { api.joinInvite(token, code) }
        catch (error: CancellationException) { throw error }
        catch (error: Throwable) { if (request == accountGeneration) failed(inviteError(error)); return@launchAction }
        if (request != accountGeneration) return@launchAction
        val list = try { api.spaces(token) } catch (error: CancellationException) { throw error } catch (_: Throwable) { null }
        if (request != accountGeneration) return@launchAction
        val current = mutable.value
        mutable.value = current.copy(
            spaces = list?.spaces?.takeIf { spaces -> spaces.any { it.id == result.space.id } }
                ?: (current.spaces.filter { it.id != result.space.id } + result.space),
            invitations = (list?.invitations ?: current.invitations).filter { it.id != result.space.id },
            limits = list?.limits ?: current.limits,
        )
        done(); selectSpace(result.space.id, result.channelId)
    }
    private fun inviteError(error: Throwable): String =
        if (error is ApiException && error.status == 404) INVALID_INVITE_LINK else message(error)
    fun acceptInvitation(invitation: Space, done: () -> Unit = {}) = launchAction { request ->
        val accepted = api.acceptSpaceInvitation(requireAccountToken(), invitation.id)
        if (request != accountGeneration || mutable.value.invitations.none { it.id == invitation.id }) return@launchAction
        mutable.value = mutable.value.copy(
            invitations = mutable.value.invitations.filter { it.id != invitation.id },
            spaces = mutable.value.spaces.filter { it.id != accepted.id } + accepted,
        )
        done(); selectSpace(accepted.id)
    }
    fun declineInvitation(invitation: Space, done: () -> Unit = {}) = launchAction { request ->
        api.declineSpaceInvitation(requireAccountToken(), invitation.id)
        if (request != accountGeneration || mutable.value.invitations.none { it.id == invitation.id }) return@launchAction
        mutable.value = mutable.value.copy(invitations = mutable.value.invitations.filter { it.id != invitation.id })
        done()
    }
    /** [done] runs only after the removal succeeds, so a failed one keeps its confirmation open. */
    fun removeSpaceMember(member: Member, done: () -> Unit = {}) = launchAction { request ->
        val detail = requireNotNull(mutable.value.selectedSpace)
        val context = AdminMutationContext(request, detail.space.id)
        api.removeSpaceMember(requireAccountToken(), detail.space.id, member.id)
        if (!context.isCurrent(accountGeneration, mutable.value.selectedSpace)) return@launchAction
        replaceDetail(detail.copy(members = detail.members.filter { it.id != member.id }))
        done()
    }
    /** Starts empty for [channel]: another channel's grants never show, and their late reply is dropped. */
    fun loadChannelGrants(channel: Channel) = launchAction { request ->
        mutable.value = mutable.value.copy(channelGrantsChannelId = channel.id, channelGrants = emptyList(), pendingChannelInvitations = emptyList())
        val detail = requireNotNull(mutable.value.selectedSpace)
        val context = AdminMutationContext(request, detail.space.id, channel.id)
        val response = if (channel.private) api.channelMembers(requireAccountToken(), detail.space.id, channel.id) else MemberList(emptyList())
        if (!context.isCurrent(accountGeneration, mutable.value.selectedSpace)) return@launchAction
        mutable.value = mutable.value.withChannelGrants(channel.id) { _, _ -> response.members to response.invitations }
    }
    fun addChannelGrant(channel: Channel, username: String, done: () -> Unit = {}, failed: (String) -> Unit = {}) = launchAction { request ->
        val detail = requireNotNull(mutable.value.selectedSpace)
        val context = AdminMutationContext(request, detail.space.id, channel.id)
        val member = memberRequest(request, failed) { api.addChannelMember(requireAccountToken(), detail.space.id, channel.id, username) } ?: return@launchAction
        if (!context.isCurrent(accountGeneration, mutable.value.selectedSpace)) return@launchAction
        mutable.value = mutable.value.withChannelGrants(channel.id) { members, invitations -> members to invitations.filter { it.id != member.id } + member }
        done()
    }

    /** Web's MemberManager: an add's failure belongs to its form, which keeps the username, not to the screen. */
    private suspend fun <T> memberRequest(request: Long, failed: (String) -> Unit, block: suspend () -> T): T? = try { block() }
        catch (error: CancellationException) { throw error }
        catch (error: Throwable) { if (request == accountGeneration) failed(error.message ?: "That request did not work."); null }

    /** Like [memberRequest], with the readable error text: invite-link forms keep their choices on failure. */
    private suspend fun <T> formRequest(request: Long, failed: (String) -> Unit, block: suspend () -> T): T? = try { block() }
        catch (error: CancellationException) { throw error }
        catch (error: Throwable) { if (request == accountGeneration) failed(message(error)); null }

    /** [done] runs only after the removal succeeds, so a failed one keeps its confirmation open. */
    fun removeChannelGrant(channel: Channel, member: Member, done: () -> Unit = {}) = launchAction { request ->
        val detail = requireNotNull(mutable.value.selectedSpace)
        val context = AdminMutationContext(request, detail.space.id, channel.id)
        api.removeChannelMember(requireAccountToken(), detail.space.id, channel.id, member.id)
        if (!context.isCurrent(accountGeneration, mutable.value.selectedSpace)) return@launchAction
        mutable.value = mutable.value.withChannelGrants(channel.id) { members, invitations -> members.filter { it.id != member.id } to invitations }
        done()
    }

    fun cancelChannelInvitation(channel: Channel, member: Member) = launchAction { request ->
        val detail = requireNotNull(mutable.value.selectedSpace)
        val context = AdminMutationContext(request, detail.space.id, channel.id)
        api.removeChannelMember(requireAccountToken(), detail.space.id, channel.id, member.id)
        if (context.isCurrent(accountGeneration, mutable.value.selectedSpace))
            mutable.value = mutable.value.withChannelGrants(channel.id) { members, invitations -> members to invitations.filter { it.id != member.id } }
    }

    fun joinChannel(channel: Channel) = mutateChannelMembership(channel) {
        api.joinChannel(requireAccountToken(), it.space.id, channel.id)
    }

    fun acceptChannelInvitation(invitation: ChannelInvitation) = mutateChannelMembership(invitation.channel) {
        api.acceptChannelInvitation(requireAccountToken(), it.space.id, invitation.channel.id)
    }

    fun declineChannelInvitation(invitation: ChannelInvitation) = launchAction { request ->
        val detail = requireNotNull(mutable.value.selectedSpace)
        api.declineChannelInvitation(requireAccountToken(), detail.space.id, invitation.channel.id)
        if (request == accountGeneration && mutable.value.selectedSpace?.space?.id == detail.space.id) refreshSpace(detail.space.id, null)
    }

    fun leaveChannel(channel: Channel) = launchAction { request ->
        val detail = requireNotNull(mutable.value.selectedSpace)
        api.leaveChannel(requireAccountToken(), detail.space.id, channel.id)
        if (request != accountGeneration || mutable.value.selectedSpace?.space?.id != detail.space.id) return@launchAction
        VoiceCallService.stopIfChannel(getApplication(), channel.id)
        val current = requireNotNull(mutable.value.selectedSpace)
        val privateLoss = channel.private && current.space.ownerId != mutable.value.account?.id
        val selected = mutable.value.selectedChannel?.id
        replaceDetail(current.copy(channels = current.channels.mapNotNull {
            if (it.id != channel.id) it else if (privateLoss) null else it.copy(joined = false)
        }))
        // Leaving a channel gives up its staged files: they could no longer be sent there.
        dropChannelDrafts(channel.id)
        if (selected == channel.id) { ++generation; closeChannel(clearPending = true) }
        refreshSpace(detail.space.id, if (selected == channel.id && privateLoss) null else selected)
    }

    private fun mutateChannelMembership(channel: Channel, mutation: suspend (SpaceDetail) -> Channel) = launchAction { request ->
        val detail = requireNotNull(mutable.value.selectedSpace)
        mutation(detail)
        if (request == accountGeneration && mutable.value.selectedSpace?.space?.id == detail.space.id) refreshSpace(detail.space.id, channel.id)
    }

    private suspend fun refreshSpace(spaceId: String, preferredChannelId: String?) {
        val account = accountGeneration
        val navigation = generation
        val since = profileEventCount
        val detail = api.space(requireAccountToken(), spaceId)
        if (accountGeneration != account || generation != navigation || mutable.value.selectedSpace?.space?.id != spaceId) return
        ++spaceAccessGeneration
        val preferred = detail.channels.firstOrNull { it.id == preferredChannelId }
        val fallback = detail.channels.firstOrNull { it.joined }
        replaceDetail(detail)
        repairProfiles(detail.members.map { it.profile() }, since)
        (preferred ?: fallback)?.let(::selectChannel) ?: closeChannel(clearPending = true)
    }

    private fun replaceDetail(detail: SpaceDetail) {
        conversationHistories.entries.removeAll { entry ->
            entry.value.space?.id == detail.space.id && detail.channels.none { it.id == entry.key }
        }
        if (openingConversationId != null && mutable.value.selectedSpace?.space?.id == detail.space.id &&
            detail.channels.none { it.id == openingConversationId } && mutable.value.directConversations.none { it.id == openingConversationId }) cancelConversationOpening()
        mutable.value = mutable.value.copy(
            selectedSpace = detail,
            spaces = mutable.value.spaces.map { if (it.id == detail.space.id) detail.space else it },
            voiceRosters = mutable.value.voiceRosters.filterKeys { id -> id.isEmpty() && detail.space.demo || detail.channels.any { it.id == id } },
            voiceSessionStartedAt = mutable.value.voiceSessionStartedAt.filterKeys { id -> id.isEmpty() && detail.space.demo || detail.channels.any { it.id == id } },
            deniedVoiceChannels = mutable.value.deniedVoiceChannels.filterTo(mutableSetOf()) { id -> detail.channels.any { it.id == id } },
        )
        val selected = mutable.value.selectedChannel
        if (selected != null && !selected.direct && selected.spaceId == detail.space.id && mutable.value.visit == null &&
            detail.channels.none { it.id == selected.id }) revokeChannel()
        gateway?.watchMedia(mediaChannels(detail), detail.space.demo)
        watchVisiblePresence()
    }

    /** Whether a chat request lost access to the open channel. A DM privacy or block refusal
     * keeps the conversation with an inline error instead. Only a 401 means the chat session
     * itself is invalid, so only it retires the session for the next action to replace. */
    private fun lostChannel(error: Throwable): Boolean {
        if (error !is ApiException || directMessageError(error.code) != null) return false
        if (error.status == 401) chatToken = null
        return error.status in listOf(401, 403, 404)
    }

    /** Shows Home: its first conversation is ready, failed, or took longer than the cap. */
    private fun finishRestoring() {
        if (mutable.value.restoring) mutable.value = mutable.value.copy(restoring = false)
    }

    private fun revokeChannel() {
        mutable.value.selectedChannel?.id?.let { conversationHistories.remove(it) }
        mutable.value.selectedChannel?.id?.let { VoiceCallService.stopIfChannel(getApplication(), it) }
        // A visit that lost access returns to the Feedback inbox.
        val visiting = mutable.value.visit != null && mutable.value.account?.feedbackTeam == true
        ++generation
        mutable.value.selectedChannel?.id?.let { dropChannelDrafts(it) }
        closeChannel(clearPending = true)
        mutable.value = mutable.value.copy(error = "You no longer have access to this channel.", feedbackShown = visiting)
    }

    private fun closeChannel(clearPending: Boolean) {
        cancelConversationOpening()
        ++historyAnchorRequest
        gateway?.close(); gateway = null
        uploadSend?.cancel()
        ++threadRequest
        threadPages.clear()
        ++threadUnreadRevision
        threadUnreadJob?.cancel(); threadUnreadJob = null
        mutable.value = mutable.value.copy(unreadThreads = emptyMap())
        durableReplayCursor = null
        refreshingHistory = false
        gatewayStatus?.cancel(); gatewayStatus = null
        typingExpiry?.cancel(); typingExpiry = null; typers.clear()
        // A new conversation starts quiet; the old stream's typing expires on its own.
        typingIdle?.cancel(); typingIdle = null; typingActive = false; typingSent = false
        unloadedReactions.clear()
        unloadedAttachments.clear()
        if (clearPending) {
            pendingSends.clear()
            // Regular and thread files retain their original destination while uploads carry on.
            // An uncertain send may still commit; do not cancel its files.
            pendingDrafts.forEach { discardDraftFiles(it.key) }
            pendingDrafts = emptyList()
            urlBatch?.cancel(); urlBatch = null; urlQueue.clear(); urlRefresh.reset()
            mutable.value = mutable.value.copy(
                freshAttachmentUrls = emptyMap(), localAttachmentPreviews = emptyMap(),
            )
        }
        reactionWorkers.values.forEach { it.cancel() }
        reactionWorkers.clear(); reactionIntents.clear(); authoritativeReactionMessages.clear()
        reactorCache.clear()
        pinWorkers.values.forEach { it.cancel() }; pinWorkers.clear(); pinSnapshots.clear()
        forwardSnapshots.clear()
        editSnapshots.clear()
        pinSnapshotCursor = null
        mutable.value = mutable.value.copy(
            selectedChannel = null, selectedDirectId = null, messages = emptyList(), typingAuthors = emptyList(), presence = emptyMap(),
            channelVisitUnread = null,
            visit = null, feedbackShown = false,
            thread = null, threadOnlyRows = emptySet(), replyTargets = emptyMap(),
            contextStart = null, contextEnd = null, hasNewerMessages = false, loadingNewer = false,
            focusedMessageId = null, loadingMessageContext = false, messageContextError = null,
            loadingOlder = false, olderError = null, messagesLoading = false, messagesError = null, refreshError = null,
            voiceRosters = emptyMap(),
            voiceSessionStartedAt = emptyMap(),
            gateway = GatewayStatus.DISCONNECTED, pendingMessage = if (clearPending) null else mutable.value.pendingMessage,
            reactionSaves = emptyMap(), attachmentProgress = emptyMap(),
            pinnedMessages = emptyList(), pinSaves = emptyMap(),
            pinIntents = emptyMap(), editIntents = emptyMap(),
        )
    }

    private fun invalidate() { ++generation; closeChannel(clearPending = true) }
    private fun requireAccountToken() = checkNotNull(accountToken) { "Sign in required." }
    private fun fail(error: Throwable) { mutable.value = mutable.value.copy(busy = false, error = message(error)) }
    // Display text only: logic compares ApiException's status, code and message, never this.
    private fun message(error: Throwable) = (error as? ApiException)?.code?.let(::directMessageError) ?: friendlyError(error)
    fun clearError() { mutable.value = mutable.value.copy(error = null) }

    private fun launchAccountAction(block: suspend (Long) -> Unit) = viewModelScope.launch {
        val request = accountGeneration
        mutable.value = mutable.value.copy(busy = true, error = null)
        try { block(request) } catch (error: Throwable) {
            if (request == accountGeneration) mutable.value = mutable.value.copy(busy = false, error = accountMessage(error))
        }
        finally { if (request == accountGeneration) mutable.value = mutable.value.copy(busy = false) }
    }
    private fun launchAction(block: suspend (Long) -> Unit) = viewModelScope.launch {
        val request = accountGeneration
        mutable.value = mutable.value.copy(busy = true, error = null)
        membershipChanges++
        try { block(request) } catch (error: Throwable) { if (request == accountGeneration) fail(error) }
        finally {
            membershipChanges--; membershipRevision++
            if (request == accountGeneration) mutable.value = mutable.value.copy(busy = false)
        }
    }

    override fun onCleared() {
        gateway?.close()
        profileFeed?.close()
        ForegroundConversation.id = null
        super.onCleared()
    }

    /** Web's profile save copy (`account/ProfileForm.tsx`). */
    private suspend fun <T> profileRequest(block: suspend () -> T): T = try { block() } catch (error: ApiException) {
        throw ProfileSaveException(when (error.status) {
            409 -> "That username is already taken."
            400 -> "Check the username and display name requirements."
            else -> "Your profile could not be saved. Please try again."
        })
    }

    private fun accountMessage(error: Throwable): String = if (error is ProfileSaveException) error.message else when ((error as? ApiException)?.status) {
        400 -> "Enter a valid email address."
        401 -> if ((error as ApiException).attemptsRemaining == 0) "That code can no longer be used. Request a new one."
            else "That code is incorrect or expired. Request a new one if needed."
        503 -> "Sign-in is temporarily unavailable. Please try again later."
        else -> "Something went wrong. Please try again."
    }

    private companion object {
        const val PRESENCE_PAGE_SIZE = 25
        const val TYPING_INTERVAL_MS = 500L
    }
}

/** The longest launch keeps the loading screen up while restoring the last conversation. */
internal const val RESTORE_LIMIT_MS = 5_000L

/** The server's redundant-invitation errors -> the profile card's label for that space. */
private val redundantInvitations = mapOf("user already in space" to "Member", "user already invited" to "Invited")

/** Web/Apple: [preferred] when the space has it, else the channel last opened there while still joined, else the first joined channel. */
internal fun spaceLandingChannel(channels: List<Channel>, preferred: String?, remembered: String?): Channel? =
    channels.firstOrNull { it.id == preferred } ?: channels.firstOrNull { it.id == remembered && it.joined } ?: channels.firstOrNull { it.joined }

/** The space's own channel is open (not a DM): choosing the space again changes nothing. */
internal fun showsSpace(state: AppUiState, id: String): Boolean =
    state.selectedDirectId == null && state.visit == null && !state.feedbackShown &&
        state.selectedSpace?.space?.id == id && state.selectedChannel?.spaceId == id

/** [channel] is already open the same way (joined or preview), and not waiting on Try again. */
internal fun showsChannel(state: AppUiState, channel: Channel): Boolean =
    state.selectedDirectId == null && state.visit == null && state.selectedChannel?.id == channel.id && state.selectedChannel.joined == channel.joined && state.messagesError == null && !state.messagesLoading

internal fun mergeDirects(current: List<DirectConversation>, incoming: List<DirectConversation>): List<DirectConversation> =
    (incoming + current).distinctBy { it.id }

private fun maxSeq(first: String, second: String): String =
    runCatching { maxOf(first.toBigInteger(), second.toBigInteger()).toString() }.getOrDefault(second)

/** An opened invite link: [url] as opened, its [code], and [id] to tell repeat openings apart. */
data class InviteLinkRequest(val url: String, val code: String, val id: Long)

/** Home (or Browse) shows for an account with a finished profile, past the launch loading screen. */
internal fun opensInviteLinks(state: AppUiState): Boolean {
    val account = state.account ?: return false
    return account.username != null && account.displayName != null && !state.restoring &&
        (state.screen == SessionScreen.Home || state.screen is SessionScreen.Spaces)
}

internal class ProfileSaveException(override val message: String) : Exception(message)

internal data class AdminMutationContext(val accountGeneration: Long, val spaceId: String? = null, val channelId: String? = null) {
    fun isCurrent(currentAccountGeneration: Long, selected: SpaceDetail?): Boolean =
        accountGeneration == currentAccountGeneration &&
            (spaceId == null || selected?.space?.id == spaceId) &&
            (channelId == null || selected?.channels?.any { it.id == channelId } == true)
}

internal data class VoiceNavigationContext(val generation: Long, val accountEpoch: Long, val callEpoch: Long, val spaceId: String, val channelId: String) {
    fun isCurrent(currentGeneration: Long, currentAccountEpoch: Long, currentCallEpoch: Long, voice: VoiceState): Boolean =
        generation == currentGeneration && accountEpoch == currentAccountEpoch && callEpoch == currentCallEpoch &&
            voice.spaceId == spaceId && voice.channelId == channelId &&
            voice.phase != VoiceState.Phase.IDLE && voice.phase != VoiceState.Phase.FAILED
}

internal data class VoiceDestination(val detail: SpaceDetail, val channel: Channel, val history: ChatHistory)

internal suspend fun readVoiceDestination(api: CaperApi, token: String?, spaces: List<Space>, spaceId: String, channelId: String): VoiceDestination? {
    val space = spaces.firstOrNull { it.id == spaceId } ?: return null
    val detail = api.space(checkNotNull(token), spaceId)
    if (detail.space.id != spaceId) return null
    val channel = detail.channels.firstOrNull { it.id == channelId && it.joined } ?: return null
    val history = api.history(token, channelId)
    return VoiceDestination(detail, channel, history)
}
