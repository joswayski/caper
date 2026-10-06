package chat.caper.android

import android.app.Application
import android.net.Uri
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import chat.caper.android.data.*
import chat.caper.android.model.*
import chat.caper.android.push.PushRegistration
import chat.caper.android.voice.VoiceCallService
import chat.caper.android.voice.VoiceState
import java.io.File
import java.io.IOException
import java.time.Instant
import java.util.UUID
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
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
    private val unloadedAttachments = mutableMapOf<String, AttachmentsUpdate>()
    private data class ReactionIntent(val emoji: String, val active: Boolean, val version: Long)
    private val reactionIntents = mutableMapOf<String, LinkedHashMap<String, ReactionIntent>>()
    private val reactionWorkers = mutableMapOf<String, Job>()
    private val authoritativeReactionMessages = mutableMapOf<String, ChatMessage>()
    private var reactionIntentVersion = 0L
    private var directRefresh: Job? = null
    private var pendingDirectIntent: String? = null
    private var foreground = false
    private val uploader = AttachmentUploader(api)
    /** `maxUploadBytes` from `GET /api/assets/usage`, enforced before reserving. */
    private var maxUploadBytes: Long? = null
    private val uploadJobs = mutableMapOf<String, Job>()
    /** Drafts moved into the pending message; restored by Edit, deleted once confirmed. */
    private var pendingDrafts: List<DraftAttachmentUi> = emptyList()
    private val urlRefresh = AttachmentUrlRefresh()
    private val urlQueue = linkedSetOf<String>()
    private var urlBatch: Job? = null

    init {
        loadHome()
        // Uploads now stream from the picked content; drop working copies left by older versions.
        viewModelScope.launch(Dispatchers.IO) { File(application.cacheDir, "attachment-uploads").deleteRecursively() }
        // Signed URLs live 24-48 hours; an app left open for days refreshes them before expiry.
        viewModelScope.launch { while (true) { delay(10 * 60_000L); if (foreground) refreshExpiringUrls() } }
    }

    private fun loadHome() {
        val requestAccountGeneration = accountGeneration
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
                createChatSession(requestAccountGeneration)
                if (requestAccountGeneration != accountGeneration) return@launch
                checkUploads(requestAccountGeneration)
                startDirectRefresh()
                if (PushRegistration.enabled(getApplication())) viewModelScope.launch {
                    runCatching { PushRegistration.enable(getApplication()) }
                }
                val pending = pendingDirectIntent?.let { id -> mutable.value.directConversations.firstOrNull { it.id == id } }
                if (pending != null) { pendingDirectIntent = null; selectDirect(pending) }
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
        tokens.clear()
        if (token != null) {
            viewModelScope.launch { PushRegistration.disable(getApplication(), token) }
            viewModelScope.launch { runCatching { api.logout(token) } }
        }
        loadHome()
    }

    fun openDirectFromNotification(id: String?) {
        if (id == null || !Regex("^[A-Za-z0-9]{12}$").matches(id)) return
        mutable.value.directConversations.firstOrNull { it.id == id }?.let(::selectDirect) ?: run { pendingDirectIntent = id; refreshDirectConversations() }
    }

    suspend fun canEnablePush(): Boolean {
        val token = accountToken ?: return false
        val epoch = accountGeneration
        return runCatching { "fcm" in api.pushConfig(token).platforms }.getOrDefault(false) && epoch == accountGeneration
    }

    suspend fun disablePush() { PushRegistration.disable(getApplication(), accountToken) }

    fun selectSpace(id: String) {
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
                detail.channels.firstOrNull { it.joined }?.let(::selectChannel)
            } catch (error: Throwable) {
                if (request == generation) {
                    if (error is ApiException && error.status == 404) removeUnavailableSpace(id)
                    else { retryOpen = { selectSpace(id) }; mutable.value = mutable.value.copy(busy = false, openError = message(error)) }
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
                mutable.value = mutable.value.copy(messages = history.messages, hasMoreMessages = history.hasMore, busy = false, messagesLoading = false)
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
        viewModelScope.launch {
            try {
                val history = api.history(requireAccountToken(), conversation.id)
                if (request != generation) return@launch
                require(history.channel?.direct == true) { "Direct-message history was not marked direct." }
                mutable.value = mutable.value.copy(messages = history.messages, hasMoreMessages = history.hasMore, busy = false, messagesLoading = false)
                openGateway(conversation.id, history.cursor, request)
                markDirectRead(conversation.id, history.cursor)
            } catch (error: Throwable) {
                if (request == generation) mutable.value = mutable.value.copy(busy = false, messagesLoading = false, messagesError = message(error))
            }
        }
    }

    fun startDirect(username: String, done: () -> Unit = {}) = launchAction { request ->
        val navigation = generation
        val conversation = api.startDirectConversation(requireAccountToken(), username)
        if (request != accountGeneration) return@launchAction
        mutable.value = mutable.value.copy(directConversations = mergeDirects(mutable.value.directConversations, listOf(conversation)))
        if (navigation == generation) { done(); selectDirect(conversation) }
    }

    /** Opens the signed-in account's notes, creating the real DM on first use. */
    fun openSelfDirect() {
        val account = mutable.value.account ?: return
        mutable.value.directConversations.firstOrNull { it.peer.id == account.id }?.let(::selectDirect)
            ?: account.username?.let { startDirect(it) }
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

    private fun startDirectRefresh() {
        directRefresh?.cancel()
        directRefresh = viewModelScope.launch { while (true) { delay(15_000); refreshDirectConversations() } }
    }

    fun setForeground(active: Boolean) {
        foreground = active
        if (!active) return
        refreshDirectConversations()
        refreshExpiringUrls()
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
                mutable.value = mutable.value.copy(
                    selectedSpace = destination.detail, selectedChannel = destination.channel,
                    messages = destination.history.messages, hasMoreMessages = destination.history.hasMore,
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
        val before = mutable.value.messages.firstOrNull()?.seq ?: return
        val request = generation
        if (refreshingHistory || !mutable.value.hasMoreMessages || mutable.value.loadingOlder) return
        mutable.value = mutable.value.copy(loadingOlder = true, olderError = null)
        viewModelScope.launch {
            try {
                val history = api.history(accountToken, channel.id, before)
                if (request != generation) return@launch
                val newer = authoritativeMessages()
                mutable.value = mutable.value.copy(
                    messages = projectMessages(mergeMessages(newer, history.messages, unloadedReactions, unloadedAttachments)), hasMoreMessages = history.hasMore,
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

    /** Returns false when nothing was sent, so the composer keeps its text. */
    fun send(text: String, confirmed: () -> Unit = {}): Boolean {
        val channel = mutable.value.selectedChannel?.takeIf { it.joined } ?: return false
        val author = chatAuthor ?: run {
            if (text.isBlank() && mutable.value.drafts.isEmpty()) return false
            // The session is still being created (or failed earlier): create
            // it now and send once it exists, instead of refusing the message.
            val request = generation
            val accountRequest = accountGeneration
            viewModelScope.launch {
                val session = createChatSession(accountRequest)
                if (request != generation || accountRequest != accountGeneration || mutable.value.selectedChannel?.id != channel.id) return@launch
                if (session != null && chatAuthor != null) send(text, confirmed)
                else fail(IllegalStateException("Chat session is unavailable."))
            }
            return true
        }
        // An unknown outcome must be retried with the same ID, text and files.
        val retry = pendingSends.retrying(channel.id, author)
        val drafts = mutable.value.drafts
        val files = if (retry != null) mutable.value.pendingMessage?.attachments.orEmpty() else {
            if (drafts.any { it.attachment == null }) {
                mutable.value = mutable.value.copy(attachmentError =
                    if (drafts.any { it.error != null }) "Remove files that failed to upload first." else "Wait for files to finish uploading.")
                return false
            }
            // Pending messages show the picked images until the server's message arrives.
            drafts.mapNotNull { draft -> draft.attachment?.copy(status = "ready", url = draft.thumbnail, previewUrl = draft.thumbnail) }
        }
        if (retry == null && text.isBlank() && files.isEmpty()) return false
        val request = generation
        val operation = pendingSends.begin(channel.id, author, text, files.map { it.id }, confirmed)
        if (retry == null) {
            pendingDrafts = drafts
            // While the server processes them, this device keeps showing its own picked images.
            val local = drafts.mapNotNull { draft -> draft.attachment?.id?.let { id -> draft.thumbnail?.let { id to it } } }
            mutable.value = mutable.value.copy(
                drafts = emptyList(), attachmentError = null,
                localAttachmentPreviews = (mutable.value.localAttachmentPreviews + local).entries.toList().takeLast(64).associate { it.key to it.value },
            )
        }
        mutable.value = mutable.value.copy(pendingMessage = PendingMessageUi(
            operation.id.toString(), operation.text, author, Instant.now().toString(), attachments = files,
        ))
        viewModelScope.launch {
            try {
                val capability = chatToken ?: createChatSession(accountGeneration) ?: return@launch
                val message = retryUnknownSend {
                    api.sendMessage(accountToken, capability, channel.id, author, operation.id, operation.text, operation.attachmentIds)
                }
                if (request == generation && mutable.value.selectedChannel?.id == channel.id) {
                    addMessage(message)
                    confirmPending(message)
                }
            } catch (error: Throwable) {
                if (request != generation || mutable.value.pendingMessage?.clientMessageId != operation.id.toString()) return@launch
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
    fun discardPending(restoreFiles: Boolean = false): String? {
        val pending = mutable.value.pendingMessage ?: return null
        pendingSends.clear()
        val candidates = if (restoreFiles) pendingDrafts + mutable.value.drafts else mutable.value.drafts
        val kept = candidates.take(AttachmentPolicy.MAX_ATTACHMENTS)
        (pendingDrafts + mutable.value.drafts).filter { it !in kept }.forEach { discardDraftFiles(it.key) }
        pendingDrafts = emptyList()
        mutable.value = mutable.value.copy(pendingMessage = null, drafts = kept)
        return pending.text
    }

    // --- Attachments ---------------------------------------------------------------------------

    private fun checkUploads(request: Long) {
        val token = accountToken ?: return
        viewModelScope.launch {
            val usage = try { api.assetUsage(token) } catch (cancelled: CancellationException) { throw cancelled } catch (_: Throwable) { null }
            if (request != accountGeneration) return@launch
            maxUploadBytes = usage?.maxUploadBytes?.takeIf { it > 0 }
            mutable.value = mutable.value.copy(uploadsEnabled = usage != null)
        }
    }

    /** Picked photos, videos or documents: upload the originals unchanged and hold them as composer drafts. */
    fun addAttachments(uris: List<Uri>) {
        val current = mutable.value
        val channel = current.selectedChannel?.takeIf { it.joined } ?: return
        val token = accountToken ?: return
        if (!current.uploadsEnabled || uris.isEmpty()) return
        val room = AttachmentPolicy.MAX_ATTACHMENTS - current.drafts.size
        if (room <= 0) {
            mutable.value = current.copy(attachmentError = "You can attach up to ${AttachmentPolicy.MAX_ATTACHMENTS} files.")
            return
        }
        mutable.value = current.copy(attachmentError = if (uris.size > room) "Only $room more file${if (room == 1) "" else "s"} can be attached." else null)
        val limit = maxUploadBytes
        val resolver = getApplication<Application>().contentResolver
        uris.take(room).forEach { uri ->
            val key = UUID.randomUUID().toString()
            mutable.value = mutable.value.copy(drafts = mutable.value.drafts + DraftAttachmentUi(key, "File", "file"))
            val report = progressReporter(key)
            uploadJobs[key] = viewModelScope.launch {
                try {
                    val picked = PickedContent.resolve(resolver, uri)
                    val kind = AttachmentPolicy.kind(picked.contentType)
                    updateDraft(key) {
                        it.copy(name = picked.name, kind = kind, size = picked.size, thumbnail = if (kind == "image") uri.toString() else null)
                    }
                    val attachment = uploader.upload(token, channel.id, picked, limit, report)
                    updateDraft(key) { it.copy(attachment = attachment, progress = 1f) }
                } catch (cancelled: CancellationException) {
                    throw cancelled
                } catch (error: Throwable) {
                    updateDraft(key) { it.copy(error = AttachmentPolicy.uploadErrorMessage(error)) }
                } finally {
                    uploadJobs.remove(key)
                }
            }
        }
    }

    fun removeDraft(key: String) {
        discardDraftFiles(key)
        mutable.value = mutable.value.copy(drafts = mutable.value.drafts.filter { it.key != key }, attachmentError = null)
    }

    /** Stops an unfinished upload; the API expires reservations that are never sent. */
    private fun discardDraftFiles(key: String) {
        uploadJobs.remove(key)?.cancel()
    }

    private fun updateDraft(key: String, change: (DraftAttachmentUi) -> DraftAttachmentUi) {
        mutable.value = mutable.value.copy(drafts = mutable.value.drafts.map { if (it.key == key) change(it) else it })
    }

    /** Upload progress arrives off the main thread; post at most every 1%. */
    private fun progressReporter(key: String): (Float) -> Unit {
        var last = -1f
        return { fraction ->
            if (fraction - last >= 0.01f || fraction >= 1f) {
                last = fraction
                viewModelScope.launch(Dispatchers.Main) { updateDraft(key) { if (it.attachment == null) it.copy(progress = fraction) else it } }
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
        val connection = GatewayClient(
            baseUrl = api.baseUrl, token = accountToken, channelId = channel, initialCursor = cursor,
            onMessage = { value -> viewModelScope.launch {
                if (generation == request) {
                    durableReplayCursor = value.seq
                    addMessage(value)
                }
            } },
            onReaction = { value -> viewModelScope.launch { if (generation == request) receiveReaction(value) } },
            onAttachments = { value -> viewModelScope.launch { if (generation == request) receiveAttachments(value) } },
            onAttachmentProgress = { value -> viewModelScope.launch { if (generation == request) receiveAttachmentProgress(value) } },
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
        mutable.value = mutable.value.copy(typingAuthors = typers.values.filter { it.typing && it.author.id != chatAuthor?.id }.map { it.author })
        typingExpiry?.cancel()
        val next = typers.values.minOfOrNull { it.expiresAt } ?: return
        typingExpiry = viewModelScope.launch { delay((next - now).coerceAtLeast(1)); refreshTypers() }
    }

    private fun addMessage(message: ChatMessage) {
        if (message.channelId != mutable.value.selectedChannel?.id) return
        val messages = authoritativeMessages()
        val isNew = messages.none { it.id == message.id }
        mutable.value = mutable.value.copy(messages = projectMessages(mergeMessages(messages, listOf(message), unloadedReactions, unloadedAttachments)))
        if (isNew) {
            // Web chimes for someone else's new message in the open conversation.
            if (message.author.id != chatAuthor?.id) chat.caper.android.ui.CaperEffects.play(chat.caper.android.ui.CaperEffects.Effect.Message)
        }
        confirmPending(message)
        if (mutable.value.selectedDirectId == message.channelId) markDirectRead(message.channelId, message.seq)
    }

    private fun receiveReaction(update: ReactionUpdate, sequenced: Boolean = true) {
        if (update.channelId != mutable.value.selectedChannel?.id) return
        durableReplayCursor = replayCursorAfterReaction(durableReplayCursor, update, sequenced)
        if (sequenced && mutable.value.selectedDirectId == update.channelId) markDirectRead(update.channelId, update.seq)
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

    /** `message.attachments`: sequenced exactly like reactions, replacing the message's files. */
    private fun receiveAttachments(update: AttachmentsUpdate) {
        if (update.channelId != mutable.value.selectedChannel?.id) return
        durableReplayCursor = update.seq
        if (mutable.value.selectedDirectId == update.channelId) markDirectRead(update.channelId, update.seq)
        if (mutable.value.messages.none { it.id == update.messageId }) {
            if (!cacheUnseenAttachments(unloadedAttachments, update)) mutable.value.selectedChannel?.id?.let(::resyncChannel)
            return
        }
        authoritativeReactionMessages[update.messageId]?.let { authoritativeReactionMessages[update.messageId] = mergeAttachments(it, update) }
        val settled = update.attachments.filter { it.state != AttachmentState.PROCESSING }.map { it.id }.toSet()
        mutable.value = mutable.value.copy(
            messages = mutable.value.messages.map { mergeAttachments(it, update) },
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

    private fun confirmPending(message: ChatMessage) {
        pendingSends.confirm(message)?.let {
            mutable.value = mutable.value.copy(pendingMessage = null)
            pendingDrafts.forEach { draft -> discardDraftFiles(draft.key) }
            pendingDrafts = emptyList()
            it.confirmed.invoke()
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
            selectedChannel = channel, selectedDirectId = previous.selectedDirectId,
            messages = previous.messages, hasMoreMessages = previous.hasMoreMessages,
            gateway = GatewayStatus.CONNECTING, busy = true, error = null,
        )
        viewModelScope.launch {
            try {
                val history = api.history(accountToken, channel.id)
                if (generation != request || mutable.value.selectedChannel?.id != channel.id) return@launch
                history.messages.forEach(::confirmPending)
                val recovered = recoverHistory(authoritativeMessages(), previous.hasMoreMessages, previousCursor, history)
                mutable.value = mutable.value.copy(
                    messages = projectMessages(recovered.messages), hasMoreMessages = recovered.hasMore, busy = false,
                )
                openGateway(channel.id, history.cursor, request, channel.joined)
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
        unloadedAttachments.clear()
        if (clearPending) {
            pendingSends.clear()
            // Uploads belong to one conversation; abandon them when it changes.
            (pendingDrafts + mutable.value.drafts).forEach { discardDraftFiles(it.key) }
            pendingDrafts = emptyList()
            urlBatch?.cancel(); urlBatch = null; urlQueue.clear(); urlRefresh.reset()
            mutable.value = mutable.value.copy(
                drafts = emptyList(), attachmentError = null, freshAttachmentUrls = emptyMap(), localAttachmentPreviews = emptyMap(),
            )
        }
        reactionWorkers.values.forEach { it.cancel() }
        reactionWorkers.clear(); reactionIntents.clear(); authoritativeReactionMessages.clear()
        mutable.value = mutable.value.copy(
            selectedChannel = null, selectedDirectId = null, messages = emptyList(), typingAuthors = emptyList(), presence = emptyMap(),
            loadingOlder = false, olderError = null, messagesLoading = false, messagesError = null, refreshError = null,
            voiceRosters = emptyMap(),
            voiceSessionStartedAt = emptyMap(),
            gateway = GatewayStatus.DISCONNECTED, pendingMessage = if (clearPending) null else mutable.value.pendingMessage,
            reactionSaves = emptyMap(), attachmentProgress = emptyMap(),
        )
    }

    private fun invalidate() { ++generation; closeChannel(clearPending = true) }
    private fun requireAccountToken() = checkNotNull(accountToken) { "Sign in required." }
    private fun fail(error: Throwable) { mutable.value = mutable.value.copy(busy = false, error = message(error)) }
    private fun message(error: Throwable) = error.message ?: "That request did not work."
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

    override fun onCleared() { gateway?.close(); super.onCleared() }

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
