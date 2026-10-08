package chat.caper.android

import android.Manifest
import android.app.Application
import android.content.pm.PackageManager
import android.os.Build
import androidx.core.content.ContextCompat
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import chat.caper.android.data.*
import chat.caper.android.model.*
import chat.caper.android.push.CaperNotifications
import chat.caper.android.push.ForegroundConversation
import chat.caper.android.push.PushOffer
import chat.caper.android.push.PushRegistration
import chat.caper.android.push.pushOffer
import chat.caper.android.voice.VoiceCallService
import chat.caper.android.voice.VoiceState
import java.io.IOException
import java.time.Instant
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.launch
import kotlinx.coroutines.withTimeoutOrNull

class CaperViewModel(application: Application) : AndroidViewModel(application) {
    private val api = CaperApi()
    private val tokens = TokenStore(application)
    private val mutable = MutableStateFlow(AppUiState())
    val state: StateFlow<AppUiState> = mutable.asStateFlow()
    private var accountToken: String? = null
    private var chatToken: String? = null
    private var chatAuthor: ChatAuthor? = null
    private var gateway: GatewayClient? = null
    private var gatewayStatus: Job? = null
    private var typingExpiry: Job? = null
    private val typers = mutableMapOf<String, TypingAuthor>()
    private var generation = 0L
    private var durableReplayCursor: String? = null
    private var refreshingHistory = false
    private var accountGeneration = 0L
    private var spaceAccessGeneration = 0L
    internal val accountEpoch: Long get() = accountGeneration
    internal val spaceAccessEpoch: Long get() = spaceAccessGeneration
    private var voiceAuthorizationRequest = 0L
    private val pendingSends = PendingSendTracker()
    private val unloadedReactions = mutableMapOf<String, ReactionUpdate>()
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
    private var peopleRefresh: Job? = null
    private var pendingDirectIntent: String? = null
    private var pendingChannelIntent: Pair<String, String>? = null
    private var notificationEdits: NotificationEdits? = null
    private var foreground = false
    private var threadRequest = 0L

    init {
        loadHome()
        viewModelScope.launch { state.map { it.selectedChannel?.id }.distinctUntilChanged().collect { publishForegroundConversation() } }
    }

    private fun loadHome() {
        val requestAccountGeneration = accountGeneration
        notificationEdits = null
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
                    screen = SessionScreen.Home, account = account,
                    spaces = list.spaces, invitations = list.invitations, limits = list.limits,
                    directConversations = directs.getOrNull()?.conversations.orEmpty(),
                    error = directs.exceptionOrNull()?.let(::message),
                )
                refreshBlocks()
                refreshNotificationSettings()
                createChatSession(requestAccountGeneration)
                if (requestAccountGeneration != accountGeneration) return@launch
                startDirectRefresh()
                if (PushRegistration.enabled(getApplication())) viewModelScope.launch {
                    runCatching { PushRegistration.enable(getApplication()) }
                } else offerPush(account.id, requestAccountGeneration)
                val pending = pendingDirectIntent?.let { id -> mutable.value.directConversations.firstOrNull { it.id == id } }
                val pendingChannel = pendingChannelIntent?.takeIf { (space, _) -> list.spaces.any { it.id == space } }
                pendingChannelIntent = null
                if (pending != null) { pendingDirectIntent = null; selectDirect(pending) }
                else if (pendingChannel != null) selectSpace(pendingChannel.first, pendingChannel.second)
                else list.spaces.firstOrNull()?.let { selectSpace(it.id) }
            } catch (error: Throwable) {
                if (requestAccountGeneration == accountGeneration) {
                    mutable.value = AppUiState(screen = SessionScreen.SignedOut, error = message(error))
                }
            }
        }
    }

    fun showLogin() { mutable.value = mutable.value.copy(screen = SessionScreen.SignedOut, error = null) }

    fun requestCode(email: String) = launchAccountAction { request ->
        val challenge = api.requestCode(email)
        if (request == accountGeneration) mutable.value = mutable.value.copy(screen = SessionScreen.Verify(challenge.challengeId, email))
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
        val account = profileRequest { api.profile(requireAccountToken(), username, displayName) }
        if (request != accountGeneration) return@launchAction
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
        accountToken = null
        chatToken = null
        chatAuthor = null
        directRefresh?.cancel(); directRefresh = null
        peopleRefresh?.cancel(); peopleRefresh = null
        tokens.clear()
        notificationEdits = null
        if (token != null) {
            viewModelScope.launch { PushRegistration.disable(getApplication(), token) }
            viewModelScope.launch { runCatching { api.logout(token) } }
        }
        loadHome()
    }

    /** A notification tap: a DM by [conversationId], or a channel by [spaceId] and [channelId]. */
    fun openFromNotification(conversationId: String?, spaceId: String?, channelId: String?) {
        val id = Regex("^[A-Za-z0-9]{12}$")
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
            catch (error: Throwable) { error.message ?: "Notifications could not be enabled." }
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

    /** Opens a space at [preferredChannelId] when it has one, otherwise its first joined channel. */
    fun selectSpace(id: String, preferredChannelId: String? = null) {
        if (mutable.value.spaces.none { it.id == id }) return
        ++spaceAccessGeneration
        val request = ++generation
        closeChannel(clearPending = true)
        mutable.value = mutable.value.copy(deniedVoiceChannels = emptySet())
        mutable.value = mutable.value.copy(busy = true, error = null, openError = null, pendingSpaceInvitations = emptyList())
        viewModelScope.launch {
            try {
                val detail = api.space(requireAccountToken(), id)
                if (request != generation) return@launch
                mutable.value = mutable.value.copy(selectedSpace = detail, busy = false, presencePage = 0)
                (detail.channels.firstOrNull { it.id == preferredChannelId } ?: detail.channels.firstOrNull { it.joined })?.let(::selectChannel)
            } catch (error: Throwable) {
                if (request == generation) {
                    if (error is ApiException && error.status == 404) removeUnavailableSpace(id)
                    else { retryOpen = { selectSpace(id, preferredChannelId) }; mutable.value = mutable.value.copy(busy = false, openError = message(error)) }
                }
            }
        }
    }

    private fun removeUnavailableSpace(id: String) {
        VoiceCallService.stopIfSpace(getApplication(), id)
        ++spaceAccessGeneration
        closeChannel(clearPending = true)
        mutable.value = mutable.value.copy(
            spaces = mutable.value.spaces.filter { it.id != id }, selectedSpace = null,
            busy = false, openError = null, error = "This space is no longer available.",
        )
    }

    private var retryOpen: (() -> Unit)? = null

    /** Web's Retry opening: repeats the space selection that failed. */
    fun retryOpening() { retryOpen?.invoke() }

    fun selectChannel(channel: Channel) {
        val request = ++generation
        closeChannel(clearPending = true)
        mutable.value = mutable.value.copy(selectedChannel = channel, selectedDirectId = null, messages = emptyList(), busy = true, error = null, messagesLoading = true)
        viewModelScope.launch {
            try {
                val history = api.history(accountToken, channel.id)
                if (request != generation) return@launch
                installHistoryPins(history)
                mutable.value = mutable.value.copy(messages = mergeTimelinePins(history.messages), hasMoreMessages = history.hasMore, busy = false, messagesLoading = false)
                openGateway(channel.id, history.cursor, request, channel.joined)
            } catch (error: Throwable) {
                if (request != generation) return@launch
                if (error is ApiException && error.status in listOf(401, 403, 404)) revokeChannel()
                // Web shows a failed first load in the conversation with Try again.
                else mutable.value = mutable.value.copy(busy = false, messagesLoading = false, messagesError = message(error))
            }
        }
    }

    fun selectDirect(conversation: DirectConversation) {
        val request = ++generation
        closeChannel(clearPending = true)
        val channel = Channel(conversation.id, "", conversation.peer.displayName, private = true, direct = true)
        mutable.value = mutable.value.copy(selectedChannel = channel, selectedDirectId = conversation.id, messages = emptyList(), busy = true, error = null, messagesLoading = true)
        refreshPeople()
        viewModelScope.launch {
            try {
                val history = api.history(requireAccountToken(), conversation.id)
                if (request != generation) return@launch
                require(history.channel?.direct == true) { "Direct-message history was not marked direct." }
                installHistoryPins(history)
                mutable.value = mutable.value.copy(messages = mergeTimelinePins(history.messages), hasMoreMessages = history.hasMore, busy = false, messagesLoading = false)
                openGateway(conversation.id, history.cursor, request)
                markDirectRead(conversation.id, history.cursor)
            } catch (error: Throwable) {
                if (request == generation) mutable.value = mutable.value.copy(busy = false, messagesLoading = false, messagesError = message(error))
            }
        }
    }

    fun startDirect(username: String, done: () -> Unit = {}) = launchAction { request -> openDirectByUsername(username, request, done) }

    /** `POST /api/dms` by username (it returns an existing DM), then opens it unless you navigated meanwhile. */
    private suspend fun openDirectByUsername(username: String, request: Long, done: () -> Unit) {
        val navigation = generation
        val conversation = api.startDirectConversation(requireAccountToken(), username)
        if (request != accountGeneration) return
        mutable.value = mutable.value.copy(directConversations = mergeDirects(mutable.value.directConversations, listOf(conversation)))
        if (navigation == generation) { done(); selectDirect(conversation) }
    }

    /**
     * The mention card's Message: opens a loaded DM with that person, otherwise the
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
            try { openDirectByUsername(username, request, done) }
            catch (error: CancellationException) { throw error }
            catch (error: Throwable) { if (request == accountGeneration) failed(message(error)) }
        }
    }

    /** Opens the signed-in account's notes, creating the real DM on first use. */
    fun openSelfDirect() {
        val account = mutable.value.account ?: return
        mutable.value.directConversations.firstOrNull { it.peer.id == account.id }?.let(::selectDirect)
            ?: account.username?.let { startDirect(it) }
    }

    /** Refreshes DM mention candidates; the previous list (or the DM peer) stays until it succeeds. */
    private fun refreshPeople() {
        val token = accountToken ?: return
        val request = accountGeneration
        peopleRefresh?.cancel()
        peopleRefresh = viewModelScope.launch {
            runCatching { api.people(token) }.onSuccess { result ->
                if (request == accountGeneration) mutable.value = mutable.value.copy(people = result.people)
            }
        }
    }

    fun refreshDirectConversations() {
        val token = accountToken ?: return
        val request = accountGeneration
        viewModelScope.launch {
            runCatching { api.directConversations(token) }.onSuccess { result ->
                if (request == accountGeneration) {
                    mutable.value = mutable.value.copy(directConversations = result.conversations)
                    pendingDirectIntent?.let { id -> result.conversations.firstOrNull { it.id == id } }?.let {
                        pendingDirectIntent = null
                        selectDirect(it)
                    }
                }
            }
        }
    }

    fun setRequestsOpen(open: Boolean) { mutable.value = mutable.value.copy(requestsOpen = open) }

    /** Accept: the request joins the main list and the composer replaces the request bar. */
    fun acceptRequest(conversation: DirectConversation, failed: (String) -> Unit) = accountRequest(failed) { token, request ->
        val accepted = api.acceptDirectRequest(token, conversation.id)
        if (request != accountGeneration) return@accountRequest
        val directs = mutable.value.directConversations.map { if (it.id == conversation.id) accepted.copy(lastSeq = maxSeq(it.lastSeq, accepted.lastSeq)) else it }
        mutable.value = mutable.value.copy(directConversations = directs, requestsOpen = messageRequests(directs).isNotEmpty() && mutable.value.requestsOpen)
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
        val directs = mutable.value.directConversations.filter { it.id != id }
        mutable.value = mutable.value.copy(directConversations = directs, requestsOpen = messageRequests(directs).isNotEmpty())
        if (mutable.value.selectedDirectId != id) return
        val channel = mutable.value.selectedSpace?.channels?.firstOrNull { it.joined }
        if (channel != null) selectChannel(channel) else invalidate()
    }

    fun refreshBlocks() {
        val token = accountToken ?: return
        val request = accountGeneration
        viewModelScope.launch {
            try {
                val blocks = api.blocks(token).blocks
                if (request == accountGeneration) mutable.value = mutable.value.copy(blocks = blocks, blocksError = null)
            } catch (error: CancellationException) { throw error }
            catch (error: Throwable) { if (request == accountGeneration) mutable.value = mutable.value.copy(blocksError = message(error)) }
        }
    }

    /** Who a message's author is, for blocking: the best name this client already has. */
    fun blockTarget(author: ChatAuthor): BlockedAccount {
        val state = mutable.value
        state.selectedSpace?.members?.firstOrNull { it.id == author.id }?.let { return BlockedAccount(it.id, it.username, it.displayName, it.avatarId) }
        state.directConversations.firstOrNull { it.peer.id == author.id }?.peer?.let { return BlockedAccount(it.id, it.username, it.displayName, it.avatarId) }
        return BlockedAccount(author.id, "", author.name, author.avatarId)
    }

    /** Blocks [account] everywhere you share; a pending request from them is declined too. */
    fun block(account: BlockedAccount, done: () -> Unit, failed: (String) -> Unit) = accountRequest(failed) { token, request ->
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
        directRefresh = viewModelScope.launch { while (true) { delay(15_000); refreshDirectConversations() } }
    }

    fun setForeground(active: Boolean) {
        foreground = active
        publishForegroundConversation()
        if (!active) return
        refreshDirectConversations()
        val current = mutable.value
        if (!current.messagesLoading) current.selectedDirectId?.let { id ->
            current.messages.lastOrNull()?.let { markDirectRead(id, it.seq) }
        }
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
        if (mutable.value.selectedSpace?.space?.id == spaceId && mutable.value.selectedChannel?.id == channelId) {
            onOpened()
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
            if (channel.direct) current.directConversations.firstOrNull { it.id == channel.id }?.let(::selectDirect)
            else selectChannel(channel)
        }
        else if (current.refreshError != null) resyncChannel(channel.id)
    }

    fun loadOlder() {
        val channel = mutable.value.selectedChannel ?: return
        val before = mutable.value.messages.firstOrNull { (it.threadRootId == null || it.broadcast) && it.id !in mutable.value.threadOnlyRows }?.seq ?: return
        val request = generation
        if (refreshingHistory || !mutable.value.hasMoreMessages || mutable.value.loadingOlder) return
        mutable.value = mutable.value.copy(loadingOlder = true, olderError = null)
        viewModelScope.launch {
            try {
                val history = api.history(accountToken, channel.id, before)
                if (request != generation) return@launch
                val newer = authoritativeMessages()
                mutable.value = mutable.value.copy(
                    messages = projectMessages(mergeTimelinePins(mergeMessages(newer, history.messages, unloadedReactions))), hasMoreMessages = history.hasMore,
                    threadOnlyRows = mutable.value.threadOnlyRows - history.messages.map { it.id }.toSet(),
                    loadingOlder = false,
                )
            } catch (error: Throwable) {
                if (request == generation) mutable.value = mutable.value.copy(loadingOlder = false, olderError = message(error))
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

    fun reportActivity() { gateway?.reportActivity() }
    fun localPresence(): String = gateway?.localPresence() ?: "offline"
    fun setTyping(active: Boolean) { chatToken?.let { gateway?.sendTyping(it, active) } }

    fun setReaction(messageId: String, emoji: String, active: Boolean) {
        val channel = mutable.value.selectedChannel?.takeIf { it.joined } ?: return
        val key = "$messageId:$emoji"
        val target = mutable.value.messages.firstOrNull { it.id == messageId } ?: return
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
                        if (error is ApiException && error.status in listOf(401, 403, 404)) revokeChannel()
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
        val own = chatAuthor ?: mutable.value.account?.let { ChatAuthor(it.id, it.displayName ?: it.username ?: "You", false, it.avatarId) } ?: return
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
                    if (error is ApiException && error.status in listOf(401, 403, 404)) revokeChannel()
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
        mutable.value = mutable.value.copy(messages = mutable.value.messages.map {
            if (it.id == messageId) projectReactionIntents(authoritative, own, intents) else it
        })
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
                val accessible = api.space(checkNotNull(token), intent.spaceId).let { detail ->
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

    fun closeThread() { ++threadRequest; mutable.value = mutable.value.copy(thread = null) }

    fun openThread(root: String) {
        mutable.value = mutable.value.copy(thread = ThreadUi(root))
        loadThread()
    }

    fun loadThread(older: Boolean = false) {
        val thread = mutable.value.thread ?: return
        val channel = mutable.value.selectedChannel?.id ?: return
        val request = ++threadRequest
        val channelRequest = generation
        mutable.value = mutable.value.copy(thread = thread.copy(loading = true, error = null))
        viewModelScope.launch {
            try {
                val page = api.thread(accountToken, channel, thread.rootId, if (older) thread.before else null)
                if (request != threadRequest || channelRequest != generation || mutable.value.thread?.rootId != thread.rootId) return@launch
                val rows = listOf(page.root) + page.messages
                val loaded = mutable.value.messages.map { it.id }.toSet()
                mutable.value = mutable.value.copy(
                    messages = projectMessages(mergeTimelinePins(mergeMessages(authoritativeMessages(), rows, unloadedReactions))),
                    threadOnlyRows = mutable.value.threadOnlyRows + rows.filter { it.id !in loaded && (it.threadRootId == null || it.broadcast) }.map { it.id },
                    thread = thread.copy(loading = false, hasMore = page.hasMore, before = page.messages.firstOrNull()?.seq ?: thread.before),
                )
            } catch (error: Throwable) {
                if (request != threadRequest || channelRequest != generation) return@launch
                if (error is ApiException && error.status in listOf(401, 403, 404)) { closeThread(); resyncChannel(channel) }
                else mutable.value = mutable.value.copy(thread = thread.copy(loading = false, error = message(error)))
            }
        }
    }

    fun send(text: String, confirmed: () -> Unit = {}, threadRootId: String? = null, broadcast: Boolean = false) {
        if (text.isBlank()) return
        if (mutable.value.pendingMessage?.let { it.threadRootId != threadRootId } == true) return
        val channel = mutable.value.selectedChannel?.takeIf { it.joined } ?: return
        val author = chatAuthor ?: run {
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
            return
        }
        val request = generation
        val operation = pendingSends.begin(channel.id, author, text, threadRootId, broadcast, confirmed)
        mutable.value = mutable.value.copy(pendingMessage = PendingMessageUi(
            operation.id.toString(), operation.text, author, Instant.now().toString(),
            threadRootId = operation.threadRootId, broadcast = operation.broadcast,
        ))
        viewModelScope.launch {
            try {
                val capability = chatToken ?: createChatSession(accountGeneration) ?: return@launch
                val message = retryUnknownSend {
                    api.sendMessage(accountToken, capability, channel.id, author, operation.id, operation.text, operation.threadRootId, operation.broadcast)
                }
                if (request == generation && mutable.value.selectedChannel?.id == channel.id) {
                    addMessage(message)
                    confirmPending(message)
                }
            } catch (error: Throwable) {
                if (request != generation || mutable.value.pendingMessage?.clientMessageId != operation.id.toString()) return@launch
                // A DM privacy or block refusal keeps the conversation; it is not a lost channel.
                if (error is ApiException && directMessageError(error.code) != null) {
                    pendingSends.definitiveFailure(operation.id)
                    mutable.value = mutable.value.copy(pendingMessage = mutable.value.pendingMessage?.copy(error = message(error), rejected = true))
                    if (error.code == DM_BLOCKED) { refreshBlocks(); refreshDirectConversations() }
                    return@launch
                }
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
    }

    fun discardPending(): String? {
        val pending = mutable.value.pendingMessage ?: return null
        pendingSends.clear()
        mutable.value = mutable.value.copy(pendingMessage = null)
        return pending.text
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
        val connection = GatewayClient(
            baseUrl = api.baseUrl, token = accountToken, channelId = channel, initialCursor = cursor,
            onMessage = { value -> viewModelScope.launch {
                if (generation == request) {
                    durableReplayCursor = value.seq
                    addMessage(value)
                }
            } },
            onReaction = { value -> viewModelScope.launch { if (generation == request) receiveReaction(value) } },
            onPin = { value -> viewModelScope.launch { if (generation == request) receivePin(value) } },
            onForward = { value -> viewModelScope.launch { if (generation == request) receiveForward(value) } },
            onEdit = { value -> viewModelScope.launch { if (generation == request) receiveEdit(value) } },
            onTyping = { author, active, revision -> viewModelScope.launch { if (participating && generation == request) receiveTyping(author, active, revision) } },
            onPresence = { snapshot -> viewModelScope.launch {
                if (participating && generation == request) mutable.value = mutable.value.copy(presence = snapshot.members.associate { it.userId to it.status })
            } },
            onMedia = { channelId, people, sessionStartedAt -> viewModelScope.launch {
                if (participating && generation == request && (channelId.isEmpty() && mutable.value.selectedSpace?.space?.demo == true ||
                    mutable.value.selectedSpace?.channels?.any { it.id == channelId } == true && channelId !in mutable.value.deniedVoiceChannels)) {
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
        if (participating && detail != null) connection.watchMedia(detail.channels.filter { it.joined }.take(24).map { it.id }, detail.space.demo)
    }

    private fun watchVisiblePresence() {
        val detail = mutable.value.selectedSpace ?: return
        if (detail.space.demo || accountToken == null || detail.members.isEmpty()) return
        val start = mutable.value.presencePage * PRESENCE_PAGE_SIZE
        val ids = detail.members.drop(start).take(PRESENCE_PAGE_SIZE).map { it.id }
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
        val messages = authoritativeMessages()
        val isNew = messages.none { it.id == message.id }
        mutable.value = mutable.value.copy(messages = projectMessages(mergeTimelinePins(mergeMessages(messages, listOf(message), unloadedReactions))))
        if (isNew) {
            // Web chimes for someone else's new message in the open conversation.
            // Never for a blocked author or an incoming request.
            val current = mutable.value
            if (message.author.id != chatAuthor?.id && !collapsesFor(message, current.blockedIds, chatAuthor?.id) && current.selectedDirect?.incoming != true)
                chat.caper.android.ui.CaperEffects.play(chat.caper.android.ui.CaperEffects.Effect.Message)
        }
        confirmPending(message)
        if (mutable.value.selectedDirectId == message.channelId) markDirectRead(message.channelId, message.seq)
    }

    private fun receiveReaction(update: ReactionUpdate, sequenced: Boolean = true) {
        if (update.channelId != mutable.value.selectedChannel?.id) return
        durableReplayCursor = replayCursorAfterReaction(durableReplayCursor, update, sequenced)
        if (sequenced && mutable.value.selectedDirectId == update.channelId) markDirectRead(update.channelId, update.seq)
        pinSnapshots[update.messageId]?.let { pinSnapshots[update.messageId] = mergeReaction(it, update) }
        mutable.value = mutable.value.copy(pinnedMessages = mutable.value.pinnedMessages.map { mergeReaction(it, update) })
        val index = mutable.value.messages.indexOfFirst { it.id == update.messageId }
        if (index < 0) {
            if (!cacheUnseenReaction(unloadedReactions, update)) {
                mutable.value.selectedChannel?.id?.let(::resyncChannel)
                return
            }
            return
        }
        val current = authoritativeReactionMessages[update.messageId]
            ?: mutable.value.messages.first { it.id == update.messageId }
        val authoritative = mergeReaction(current, update)
        authoritativeReactionMessages[update.messageId] = authoritative
        val own = mutable.value.chatAuthorId ?: mutable.value.account?.id
        val intents = reactionIntents[update.messageId]?.values.orEmpty().map { ReactionSaveUi(it.emoji, it.active) }
        mutable.value = mutable.value.copy(messages = mutable.value.messages.map {
            if (it.id == update.messageId && own != null) projectReactionIntents(authoritative, own, intents) else mergeReaction(it, update)
        })
    }

    private fun receiveEdit(update: EditUpdate) {
        if (update.channelId != mutable.value.selectedChannel?.id) return
        durableReplayCursor = update.seq
        if (mutable.value.selectedDirectId == update.channelId) markDirectRead(update.channelId, update.seq)
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

    fun canEdit(message: ChatMessage): Boolean = message.forward == null && mutable.value.selectedChannel?.let { it.id == message.channelId && it.joined } == true &&
        chatAuthor?.let { !it.isGuest && it.id == message.author.id } == true

    suspend fun editMessage(message: ChatMessage, text: String): Unit {
        check(canEdit(message)) { "Only the author can edit while participating." }
        check(message.id !in mutable.value.editIntents) { "This message is already being saved." }
        val request = generation
        val authorId = requireNotNull(chatAuthor).id
        mutable.value = mutable.value.copy(editIntents = mutable.value.editIntents + (message.id to EditIntentUi(text, message.revision)))
        try {
            val result = api.editMessage(accountToken, requireNotNull(chatToken), message.channelId, message.id, text, message.revision)
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
        if (sequenced && mutable.value.selectedDirectId == update.channelId) markDirectRead(update.channelId, update.seq)
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
    }

    private fun receiveForward(update: ForwardUpdate) {
        if (update.channelId != mutable.value.selectedChannel?.id) return
        durableReplayCursor = update.seq
        if (mutable.value.selectedDirectId == update.channelId) markDirectRead(update.channelId, update.seq)
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
        val snapshot = pinSnapshots[message.id]
        val reactions = snapshot?.let { mergeReaction(message, it) } ?: message
        val pinned = overlayEdit(overlayPin(reactions, snapshot, pinSnapshotCursor))
        forwardSnapshots[message.id]?.let { mergeForward(pinned, it) } ?: pinned
    }

    private fun confirmPending(message: ChatMessage) {
        pendingSends.confirm(message)?.let {
            mutable.value = mutable.value.copy(pendingMessage = null)
            it.confirmed.invoke()
        }
    }

    private fun resyncChannel(channelId: String) {
        val channel = mutable.value.selectedChannel?.takeIf { it.id == channelId } ?: return
        val previousCursor = durableReplayCursor ?: return
        val request = ++generation
        unloadedReactions.clear()
        val previous = mutable.value.copy(messages = authoritativeMessages())
        closeChannel(clearPending = false)
        // closeChannel normally discards channel replay state; a failed refresh must remain retryable.
        durableReplayCursor = previousCursor
        refreshingHistory = true
        // Keep the conversation readable while it reloads, as the web does.
        mutable.value = mutable.value.copy(
            selectedChannel = channel, selectedDirectId = previous.selectedDirectId,
            messages = previous.messages, hasMoreMessages = previous.hasMoreMessages,
            thread = previous.thread, threadOnlyRows = previous.threadOnlyRows,
            gateway = GatewayStatus.CONNECTING, busy = true, error = null,
        )
        viewModelScope.launch {
            try {
                val history = api.history(accountToken, channel.id)
                if (generation != request || mutable.value.selectedChannel?.id != channel.id) return@launch
                history.messages.forEach(::confirmPending)
                val recovered = recoverHistory(authoritativeMessages().filter { (it.threadRootId == null || it.broadcast) && it.id !in previous.threadOnlyRows }, previous.hasMoreMessages, previousCursor, history)
                installHistoryPins(history)
                mutable.value = mutable.value.copy(
                    messages = projectMessages(mergeTimelinePins(recovered.messages)), hasMoreMessages = recovered.hasMore, busy = false,
                    threadOnlyRows = emptySet(),
                )
                openGateway(channel.id, history.cursor, request, channel.joined)
                if (mutable.value.thread != null) loadThread()
                if (channel.direct) markDirectRead(channel.id, history.cursor)
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
    fun deleteCurrentSpace(done: () -> Unit = {}) = launchAction { request ->
        val detail = requireNotNull(mutable.value.selectedSpace)
        val context = AdminMutationContext(request, detail.space.id)
        api.deleteSpace(requireAccountToken(), detail.space.id)
        if (!context.isCurrent(accountGeneration, mutable.value.selectedSpace)) return@launchAction
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
    fun addSpaceMember(username: String) = launchAction { request ->
        val detail = requireNotNull(mutable.value.selectedSpace)
        val context = AdminMutationContext(request, detail.space.id)
        val member = api.addSpaceMember(requireAccountToken(), detail.space.id, username)
        if (!context.isCurrent(accountGeneration, mutable.value.selectedSpace)) return@launchAction
        mutable.value = mutable.value.copy(pendingSpaceInvitations = mutable.value.pendingSpaceInvitations.filter { it.id != member.id } + member)
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
    fun removeSpaceMember(member: Member) = launchAction { request ->
        val detail = requireNotNull(mutable.value.selectedSpace)
        val context = AdminMutationContext(request, detail.space.id)
        api.removeSpaceMember(requireAccountToken(), detail.space.id, member.id)
        if (!context.isCurrent(accountGeneration, mutable.value.selectedSpace)) return@launchAction
        replaceDetail(detail.copy(members = detail.members.filter { it.id != member.id }))
    }
    fun loadChannelGrants(channel: Channel) = launchAction { request ->
        val detail = requireNotNull(mutable.value.selectedSpace)
        val context = AdminMutationContext(request, detail.space.id, channel.id)
        val response = if (channel.private) api.channelMembers(requireAccountToken(), detail.space.id, channel.id) else MemberList(emptyList())
        if (!context.isCurrent(accountGeneration, mutable.value.selectedSpace)) return@launchAction
        mutable.value = mutable.value.copy(channelGrants = response.members, pendingChannelInvitations = response.invitations)
    }
    fun addChannelGrant(channel: Channel, username: String) = launchAction { request ->
        val detail = requireNotNull(mutable.value.selectedSpace)
        val context = AdminMutationContext(request, detail.space.id, channel.id)
        val member = api.addChannelMember(requireAccountToken(), detail.space.id, channel.id, username)
        if (!context.isCurrent(accountGeneration, mutable.value.selectedSpace)) return@launchAction
        mutable.value = mutable.value.copy(pendingChannelInvitations = mutable.value.pendingChannelInvitations.filter { it.id != member.id } + member)
    }
    fun removeChannelGrant(channel: Channel, member: Member) = launchAction { request ->
        val detail = requireNotNull(mutable.value.selectedSpace)
        val context = AdminMutationContext(request, detail.space.id, channel.id)
        api.removeChannelMember(requireAccountToken(), detail.space.id, channel.id, member.id)
        if (!context.isCurrent(accountGeneration, mutable.value.selectedSpace)) return@launchAction
        mutable.value = mutable.value.copy(channelGrants = mutable.value.channelGrants.filter { it.id != member.id })
    }

    fun cancelChannelInvitation(channel: Channel, member: Member) = launchAction { request ->
        val detail = requireNotNull(mutable.value.selectedSpace)
        val context = AdminMutationContext(request, detail.space.id, channel.id)
        api.removeChannelMember(requireAccountToken(), detail.space.id, channel.id, member.id)
        if (context.isCurrent(accountGeneration, mutable.value.selectedSpace))
            mutable.value = mutable.value.copy(pendingChannelInvitations = mutable.value.pendingChannelInvitations.filter { it.id != member.id })
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
        val detail = api.space(requireAccountToken(), spaceId)
        if (accountGeneration != account || generation != navigation || mutable.value.selectedSpace?.space?.id != spaceId) return
        ++spaceAccessGeneration
        val preferred = detail.channels.firstOrNull { it.id == preferredChannelId }
        val fallback = detail.channels.firstOrNull { it.joined }
        replaceDetail(detail)
        (preferred ?: fallback)?.let(::selectChannel) ?: closeChannel(clearPending = true)
    }

    private fun replaceDetail(detail: SpaceDetail) {
        mutable.value = mutable.value.copy(
            selectedSpace = detail,
            spaces = mutable.value.spaces.map { if (it.id == detail.space.id) detail.space else it },
            voiceRosters = mutable.value.voiceRosters.filterKeys { id -> id.isEmpty() && detail.space.demo || detail.channels.any { it.id == id } },
            voiceSessionStartedAt = mutable.value.voiceSessionStartedAt.filterKeys { id -> id.isEmpty() && detail.space.demo || detail.channels.any { it.id == id } },
            deniedVoiceChannels = mutable.value.deniedVoiceChannels.filterTo(mutableSetOf()) { id -> detail.channels.any { it.id == id } },
        )
        gateway?.watchMedia(detail.channels.filter { it.joined }.take(24).map { it.id }, detail.space.demo)
        watchVisiblePresence()
    }

    private fun revokeChannel() {
        mutable.value.selectedChannel?.id?.let { VoiceCallService.stopIfChannel(getApplication(), it) }
        ++generation
        closeChannel(clearPending = true)
        mutable.value = mutable.value.copy(error = "You no longer have access to this channel.")
    }

    private fun closeChannel(clearPending: Boolean) {
        gateway?.close(); gateway = null
        durableReplayCursor = null
        refreshingHistory = false
        gatewayStatus?.cancel(); gatewayStatus = null
        typingExpiry?.cancel(); typingExpiry = null; typers.clear()
        unloadedReactions.clear()
        if (clearPending) pendingSends.clear()
        reactionWorkers.values.forEach { it.cancel() }
        reactionWorkers.clear(); reactionIntents.clear(); authoritativeReactionMessages.clear()
        reactorCache.clear()
        pinWorkers.values.forEach { it.cancel() }; pinWorkers.clear(); pinSnapshots.clear()
        forwardSnapshots.clear()
        editSnapshots.clear()
        pinSnapshotCursor = null
        mutable.value = mutable.value.copy(
            selectedChannel = null, selectedDirectId = null, messages = emptyList(), typingAuthors = emptyList(), presence = emptyMap(),
            thread = null, threadOnlyRows = emptySet(),
            loadingOlder = false, olderError = null, messagesLoading = false, messagesError = null, refreshError = null,
            voiceRosters = emptyMap(),
            voiceSessionStartedAt = emptyMap(),
            gateway = GatewayStatus.DISCONNECTED, pendingMessage = if (clearPending) null else mutable.value.pendingMessage,
            reactionSaves = emptyMap(),
            pinnedMessages = emptyList(), pinSaves = emptyMap(),
            pinIntents = emptyMap(), editIntents = emptyMap(),
        )
    }

    private fun invalidate() { ++generation; closeChannel(clearPending = true) }
    private fun requireAccountToken() = checkNotNull(accountToken) { "Sign in required." }
    private fun fail(error: Throwable) { mutable.value = mutable.value.copy(busy = false, error = message(error)) }
    private fun message(error: Throwable) = (error as? ApiException)?.code?.let(::directMessageError) ?: error.message ?: "That request did not work."
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
        try { block(request) } catch (error: Throwable) { if (request == accountGeneration) fail(error) }
        finally { if (request == accountGeneration) mutable.value = mutable.value.copy(busy = false) }
    }

    override fun onCleared() {
        gateway?.close()
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
    }
}

internal fun mergeDirects(current: List<DirectConversation>, incoming: List<DirectConversation>): List<DirectConversation> =
    (incoming + current).distinctBy { it.id }

private fun maxSeq(first: String, second: String): String =
    runCatching { maxOf(first.toBigInteger(), second.toBigInteger()).toString() }.getOrDefault(second)

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
