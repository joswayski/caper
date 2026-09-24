package chat.caper.android

import android.app.Application
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import chat.caper.android.data.*
import chat.caper.android.model.*
import chat.caper.android.voice.VoiceCallService
import java.io.IOException
import java.time.Instant
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch

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
    private var accountGeneration = 0L
    internal val accountEpoch: Long get() = accountGeneration
    private val pendingSends = PendingSendTracker()

    init { loadHome() }

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
                val general = api.general()
                val room = requireNotNull(general.space) { "General space is missing." }
                val channelRoom = requireNotNull(general.channel) { "General channel is missing." }
                val list = account?.let { api.spaces(requireNotNull(accountToken)) }
                if (requestAccountGeneration != accountGeneration) return@launch
                val demo = Space(room.id, room.name, demo = true)
                val channel = Channel(channelRoom.id, room.id, channelRoom.name, false)
                val detail = SpaceDetail(demo, listOf(channel), emptyList())
                mutable.value = AppUiState(
                    screen = SessionScreen.Home, account = account,
                    spaces = listOf(demo) + (list?.spaces ?: emptyList()), limits = list?.limits,
                    selectedSpace = detail, selectedChannel = channel, messages = general.messages,
                    hasMoreMessages = general.hasMore,
                )
                openGateway(channel.id, general.cursor, ++generation)
                createChatSession(requestAccountGeneration)
            } catch (error: Throwable) {
                if (requestAccountGeneration == accountGeneration) {
                    mutable.value = AppUiState(screen = SessionScreen.Home, error = message(error))
                }
            }
        }
    }

    fun showLogin() { mutable.value = mutable.value.copy(screen = SessionScreen.SignedOut, error = null) }
    fun cancelAccountFlow() { if (mutable.value.selectedChannel != null) mutable.value = mutable.value.copy(screen = SessionScreen.Home, error = null) else loadHome() }

    fun requestCode(email: String) = launchAccountAction { request ->
        val challenge = api.requestCode(email)
        if (request == accountGeneration) mutable.value = mutable.value.copy(screen = SessionScreen.Verify(challenge.challengeId, email))
    }

    fun verify(challenge: String, code: String) = launchAccountAction { request ->
        val result = api.verifyCode(challenge, code)
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
        val account = api.profile(requireAccountToken(), username, displayName)
        if (request == accountGeneration) loadHome()
    }

    fun updateProfile(username: String, displayName: String, onSuccess: () -> Unit) = launchAction { request ->
        val account = api.profile(requireAccountToken(), username, displayName)
        if (request != accountGeneration) return@launchAction
        mutable.value = mutable.value.copy(account = account, screen = SessionScreen.Home)
        chatToken = null
        chatAuthor = null
        createChatSession(accountGeneration)
        onSuccess()
    }

    fun logout() {
        val token = accountToken
        VoiceCallService.stop(getApplication())
        ++accountGeneration
        invalidate()
        accountToken = null
        chatToken = null
        chatAuthor = null
        tokens.clear()
        if (token != null) viewModelScope.launch { runCatching { api.logout(token) } }
        loadHome()
    }

    fun selectSpace(id: String) {
        val existing = mutable.value.spaces.find { it.id == id } ?: return
        if (existing.demo) return selectDemo()
        val request = ++generation
        closeChannel(clearPending = true)
        mutable.value = mutable.value.copy(busy = true, error = null)
        viewModelScope.launch {
            try {
                val detail = api.space(requireAccountToken(), id)
                if (request != generation) return@launch
                mutable.value = mutable.value.copy(selectedSpace = detail, busy = false, presencePage = 0)
                detail.channels.firstOrNull()?.let(::selectChannel)
            } catch (error: Throwable) { if (request == generation) fail(error) }
        }
    }

    private fun selectDemo() {
        val request = ++generation
        closeChannel(clearPending = true)
        mutable.value = mutable.value.copy(busy = true, error = null)
        viewModelScope.launch {
            try {
                val history = api.general()
                if (request != generation) return@launch
                val room = requireNotNull(history.space)
                val channelRoom = requireNotNull(history.channel)
                val space = mutable.value.spaces.first { it.demo }
                val channel = Channel(channelRoom.id, room.id, channelRoom.name, false)
                mutable.value = mutable.value.copy(
                    selectedSpace = SpaceDetail(space, listOf(channel), emptyList()), selectedChannel = channel,
                    messages = history.messages, hasMoreMessages = history.hasMore, busy = false,
                )
                openGateway(channel.id, history.cursor, request)
            } catch (error: Throwable) { if (request == generation) fail(error) }
        }
    }

    fun selectChannel(channel: Channel) {
        val request = ++generation
        closeChannel(clearPending = true)
        mutable.value = mutable.value.copy(selectedChannel = channel, messages = emptyList(), busy = true, error = null)
        viewModelScope.launch {
            try {
                val history = api.history(accountToken, channel.id)
                if (request != generation) return@launch
                mutable.value = mutable.value.copy(messages = history.messages, hasMoreMessages = history.hasMore, busy = false)
                openGateway(channel.id, history.cursor, request)
            } catch (error: Throwable) {
                if (request != generation) return@launch
                if (error is ApiException && error.status in listOf(401, 403, 404)) revokeChannel() else fail(error)
            }
        }
    }

    fun loadOlder() {
        val channel = mutable.value.selectedChannel ?: return
        val before = mutable.value.messages.firstOrNull()?.seq ?: return
        val request = generation
        if (!mutable.value.hasMoreMessages || mutable.value.loadingOlder) return
        mutable.value = mutable.value.copy(loadingOlder = true, olderError = null)
        viewModelScope.launch {
            try {
                val history = api.history(accountToken, channel.id, before)
                if (request != generation) return@launch
                val newer = mutable.value.messages
                mutable.value = mutable.value.copy(
                    messages = (history.messages + newer).distinctBy { it.id }, hasMoreMessages = history.hasMore,
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

    fun reportActivity() { gateway?.reportActivity() }
    fun setTyping(active: Boolean) { chatToken?.let { gateway?.sendTyping(it, active) } }

    fun send(text: String, confirmed: () -> Unit = {}) {
        if (text.isBlank()) return
        val channel = mutable.value.selectedChannel ?: return
        val author = chatAuthor ?: return fail(IllegalStateException("Chat session is unavailable."))
        val request = generation
        val operation = pendingSends.begin(channel.id, author, text, confirmed)
        mutable.value = mutable.value.copy(pendingMessage = PendingMessageUi(
            operation.id.toString(), operation.text, author, Instant.now().toString(),
        ))
        viewModelScope.launch {
            try {
                val capability = chatToken ?: createChatSession(accountGeneration) ?: return@launch
                val message = retryUnknownSend {
                    api.sendMessage(accountToken, capability, channel.id, author, operation.id, operation.text)
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
            session.token
        } catch (error: Throwable) {
            if (requestAccountGeneration == accountGeneration) mutable.value = mutable.value.copy(error = message(error))
            null
        }
    }

    private fun openGateway(channel: String, cursor: String, request: Long) {
        val connection = GatewayClient(
            baseUrl = api.baseUrl, token = accountToken, channelId = channel, initialCursor = cursor,
            onMessage = { value -> viewModelScope.launch { if (generation == request) addMessage(value) } },
            onTyping = { author, active, revision -> viewModelScope.launch { if (generation == request) receiveTyping(author, active, revision) } },
            onPresence = { snapshot -> viewModelScope.launch {
                if (generation == request) mutable.value = mutable.value.copy(presence = snapshot.members.associate { it.userId to it.status })
            } },
            onAccessDenied = { viewModelScope.launch { if (generation == request) revokeChannel() } },
            onResync = { viewModelScope.launch { if (generation == request) resyncChannel(channel) } },
        )
        gateway = connection
        gatewayStatus = viewModelScope.launch {
            connection.status.collect { status -> if (generation == request) mutable.value = mutable.value.copy(gateway = status) }
        }
        connection.start()
        watchVisiblePresence()
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
        if (active) typers[author.id] = TypingAuthor(author, next, System.currentTimeMillis() + 6_000)
        else typers.remove(author.id)
        refreshTypers()
    }

    private fun refreshTypers() {
        val now = System.currentTimeMillis()
        typers.entries.removeAll { it.value.expiresAt <= now }
        mutable.value = mutable.value.copy(typingAuthors = typers.values.filter { it.expiresAt > now }.map { it.author })
        typingExpiry?.cancel()
        val next = typers.values.minOfOrNull { it.expiresAt } ?: return
        typingExpiry = viewModelScope.launch { delay((next - now).coerceAtLeast(1)); refreshTypers() }
    }

    private fun addMessage(message: ChatMessage) {
        if (message.channelId != mutable.value.selectedChannel?.id) return
        val messages = mutable.value.messages
        if (messages.none { it.id == message.id }) {
            mutable.value = mutable.value.copy(messages = (messages + message).sortedWith(compareBy { java.math.BigInteger(it.seq) }))
        }
        confirmPending(message)
    }

    private fun confirmPending(message: ChatMessage) {
        pendingSends.confirm(message)?.let {
            mutable.value = mutable.value.copy(pendingMessage = null)
            it.confirmed.invoke()
        }
    }

    private fun resyncChannel(channelId: String) {
        val channel = mutable.value.selectedChannel?.takeIf { it.id == channelId } ?: return
        val request = ++generation
        closeChannel(clearPending = false)
        mutable.value = mutable.value.copy(gateway = GatewayStatus.CONNECTING, busy = true, error = null)
        viewModelScope.launch {
            try {
                val history = api.history(accountToken, channel.id)
                if (generation != request || mutable.value.selectedChannel?.id != channel.id) return@launch
                history.messages.forEach(::confirmPending)
                mutable.value = mutable.value.copy(messages = history.messages, hasMoreMessages = history.hasMore, busy = false)
                openGateway(channel.id, history.cursor, request)
            } catch (error: Throwable) {
                if (generation == request) {
                    if (error is ApiException && error.status in listOf(401, 403, 404)) revokeChannel() else fail(error)
                }
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
    fun createChannel(name: String, privateChannel: Boolean, done: () -> Unit = {}) = launchAction { request ->
        val detail = requireNotNull(mutable.value.selectedSpace)
        val context = AdminMutationContext(request, detail.space.id)
        val channel = api.createChannel(requireAccountToken(), detail.space.id, name, privateChannel)
        if (!context.isCurrent(accountGeneration, mutable.value.selectedSpace)) return@launchAction
        replaceDetail(detail.copy(channels = detail.channels + channel)); done(); selectChannel(channel)
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
        channels.firstOrNull()?.let(::selectChannel) ?: closeChannel(clearPending = true)
    }
    fun addSpaceMember(username: String) = launchAction { request ->
        val detail = requireNotNull(mutable.value.selectedSpace)
        val context = AdminMutationContext(request, detail.space.id)
        val member = api.addSpaceMember(requireAccountToken(), detail.space.id, username)
        if (!context.isCurrent(accountGeneration, mutable.value.selectedSpace)) return@launchAction
        replaceDetail(detail.copy(members = detail.members.filter { it.id != member.id } + member))
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
        val grants = if (channel.private) api.channelMembers(requireAccountToken(), detail.space.id, channel.id).members else emptyList()
        if (!context.isCurrent(accountGeneration, mutable.value.selectedSpace)) return@launchAction
        mutable.value = mutable.value.copy(channelGrants = grants)
    }
    fun addChannelGrant(channel: Channel, username: String) = launchAction { request ->
        val detail = requireNotNull(mutable.value.selectedSpace)
        val context = AdminMutationContext(request, detail.space.id, channel.id)
        val member = api.addChannelMember(requireAccountToken(), detail.space.id, channel.id, username)
        if (!context.isCurrent(accountGeneration, mutable.value.selectedSpace)) return@launchAction
        mutable.value = mutable.value.copy(channelGrants = mutable.value.channelGrants.filter { it.id != member.id } + member)
    }
    fun removeChannelGrant(channel: Channel, member: Member) = launchAction { request ->
        val detail = requireNotNull(mutable.value.selectedSpace)
        val context = AdminMutationContext(request, detail.space.id, channel.id)
        api.removeChannelMember(requireAccountToken(), detail.space.id, channel.id, member.id)
        if (!context.isCurrent(accountGeneration, mutable.value.selectedSpace)) return@launchAction
        mutable.value = mutable.value.copy(channelGrants = mutable.value.channelGrants.filter { it.id != member.id })
    }

    private fun replaceDetail(detail: SpaceDetail) {
        mutable.value = mutable.value.copy(
            selectedSpace = detail,
            spaces = mutable.value.spaces.map { if (it.id == detail.space.id) detail.space else it },
        )
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
        gatewayStatus?.cancel(); gatewayStatus = null
        typingExpiry?.cancel(); typingExpiry = null; typers.clear()
        if (clearPending) pendingSends.clear()
        mutable.value = mutable.value.copy(
            selectedChannel = null, messages = emptyList(), typingAuthors = emptyList(), presence = emptyMap(),
            gateway = GatewayStatus.DISCONNECTED, pendingMessage = if (clearPending) null else mutable.value.pendingMessage,
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

    private fun accountMessage(error: Throwable): String = when ((error as? ApiException)?.status) {
        400 -> "Enter a valid email address."
        401 -> "That code is incorrect or expired. Request a new one if needed."
        503 -> "Sign-in is temporarily unavailable. Please try again later."
        else -> "Something went wrong. Please try again."
    }

    private companion object { const val PRESENCE_PAGE_SIZE = 25 }
}

internal data class AdminMutationContext(val accountGeneration: Long, val spaceId: String? = null, val channelId: String? = null) {
    fun isCurrent(currentAccountGeneration: Long, selected: SpaceDetail?): Boolean =
        accountGeneration == currentAccountGeneration &&
            (spaceId == null || selected?.space?.id == spaceId) &&
            (channelId == null || selected?.channels?.any { it.id == channelId } == true)
}
