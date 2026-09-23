package chat.caper.android

import android.app.Application
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import chat.caper.android.data.ApiException
import chat.caper.android.data.CaperApi
import chat.caper.android.data.GatewayClient
import chat.caper.android.data.PendingSendTracker
import chat.caper.android.data.SendFailure
import chat.caper.android.data.TokenStore
import chat.caper.android.data.classifySendFailure
import chat.caper.android.model.*
import chat.caper.android.voice.VoiceCallService
import java.io.IOException
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
    private var gateway: GatewayClient? = null
    private var gatewayStatus: Job? = null
    private var generation = 0L
    private var accountGeneration = 0L
    private val pendingSends = PendingSendTracker()

    init { restore() }

    private fun restore() = launchBusy { requestAccountGeneration ->
        val token = tokens.read() ?: return@launchBusy setSignedOut()
        try {
            val account = api.me(token)
            if (requestAccountGeneration != accountGeneration) return@launchBusy
            accountToken = token
            showAccount(account, requestAccountGeneration)
        } catch (error: ApiException) {
            if (error.status == 401 && requestAccountGeneration == accountGeneration) {
                tokens.clear()
                setSignedOut()
            } else throw error
        }
    }

    fun requestCode(email: String) = launchBusy { requestAccountGeneration ->
        val challenge = api.requestCode(email)
        if (requestAccountGeneration == accountGeneration) {
            mutable.value = mutable.value.copy(screen = SessionScreen.Verify(challenge.challengeId, email))
        }
    }

    fun verify(challenge: String, code: String) = launchBusy { requestAccountGeneration ->
        val result = api.verifyCode(challenge, code)
        if (requestAccountGeneration != accountGeneration) return@launchBusy
        tokens.write(result.token)
        accountToken = result.token
        showAccount(result.account, requestAccountGeneration)
    }

    fun saveProfile(username: String, displayName: String) = launchBusy { requestAccountGeneration ->
        val account = api.profile(requireToken(), username, displayName)
        if (requestAccountGeneration == accountGeneration) showAccount(account, requestAccountGeneration)
    }

    fun logout() {
        val token = accountToken
        VoiceCallService.stop(getApplication()) // local mic/peer teardown does not wait for account revocation
        ++accountGeneration
        invalidate()
        accountToken = null
        chatToken = null
        tokens.clear()
        setSignedOut()
        if (token != null) viewModelScope.launch { runCatching { api.logout(token) } }
    }

    fun selectSpace(id: String) = launchBusy { requestAccountGeneration ->
        val requestGeneration = ++generation
        closeChannel(clear = true)
        val detail = api.space(requireToken(), id)
        if (generation == requestGeneration && accountGeneration == requestAccountGeneration) {
            mutable.value = mutable.value.copy(selectedSpace = detail)
        }
    }

    fun selectChannel(channel: Channel) {
        val requestGeneration = ++generation
        closeChannel(clear = true)
        mutable.value = mutable.value.copy(selectedChannel = channel, busy = true, error = null)
        viewModelScope.launch {
            try {
                val history = api.history(requireToken(), channel.id)
                if (generation != requestGeneration) return@launch
                mutable.value = mutable.value.copy(messages = history.messages, busy = false)
                openGateway(channel.id, history.cursor, requestGeneration)
            } catch (error: Throwable) {
                if (generation != requestGeneration) return@launch
                if (error is ApiException && error.status in listOf(401, 403, 404)) revokeChannel()
                else fail(error)
            }
        }
    }

    fun backToSpaces() {
        ++generation
        closeChannel(clear = true)
        mutable.value = mutable.value.copy(selectedSpace = null)
    }

    fun backToChannels() {
        ++generation
        closeChannel(clear = true)
    }

    fun reportActivity() { gateway?.reportActivity() }

    fun send(text: String, confirmed: () -> Unit = {}) {
        if (text.isBlank()) return
        val selected = mutable.value.selectedChannel ?: return
        val account = (mutable.value.screen as? SessionScreen.Spaces)?.account ?: return
        val requestGeneration = generation
        val operation = pendingSends.begin(selected.id, account.id, text, confirmed)
        viewModelScope.launch {
            var sendIssued = false
            try {
                val token = requireToken()
                val capability = chatToken ?: api.chatSession(token, account.displayName ?: account.username ?: "Caper user").let {
                    if (generation != requestGeneration || accountToken != token) return@launch
                    require(it.author.id == account.id && !it.author.isGuest) { "Chat session identity mismatch." }
                    chatToken = it.token
                    it.token
                }
                sendIssued = true
                val message = retryUnknownSend {
                    api.sendMessage(token, capability, selected.id, account.id, operation.id, operation.text)
                }
                // HTTP confirms persistence but deliberately does not move the replay cursor.
                if (generation == requestGeneration && mutable.value.selectedChannel?.id == selected.id) {
                    addMessage(message)
                    confirmPending(message)
                }
            } catch (error: Throwable) {
                if (generation != requestGeneration) return@launch
                if (error is ApiException) {
                    when (classifySendFailure(error.status)) {
                        SendFailure.REVOKED -> { pendingSends.definitiveFailure(operation.id); revokeChannel() }
                        SendFailure.DEFINITIVE -> { pendingSends.definitiveFailure(operation.id); fail(error) }
                        SendFailure.UNKNOWN -> fail(error)
                    }
                } else {
                    if (!sendIssued && error is IllegalArgumentException) pendingSends.definitiveFailure(operation.id)
                    fail(error)
                }
            }
        }
    }

    private suspend fun retryUnknownSend(block: suspend () -> ChatMessage): ChatMessage {
        return try { block() } catch (error: IOException) {
            if (error is ApiException) throw error
            delay(250)
            block() // caller closes over the same UUID and text: server idempotency key is stable
        }
    }

    private suspend fun showAccount(account: Account, requestAccountGeneration: Long) {
        if (requestAccountGeneration != accountGeneration) return
        if (account.username == null || account.displayName == null) {
            mutable.value = AppUiState(screen = SessionScreen.Profile(account))
            return
        }
        val spaces = api.spaces(requireToken()).spaces
        if (requestAccountGeneration != accountGeneration) return
        mutable.value = AppUiState(screen = SessionScreen.Spaces(account), spaces = spaces)
    }

    private fun openGateway(channel: String, cursor: String, requestGeneration: Long) {
        val connection = GatewayClient(
            api.baseUrl, requireToken(), channel, cursor,
            onMessage = { message -> viewModelScope.launch { if (generation == requestGeneration) addMessage(message) } },
            onAccessDenied = { viewModelScope.launch { if (generation == requestGeneration) revokeChannel() } },
            onResync = { viewModelScope.launch { if (generation == requestGeneration) resyncChannel(channel) } },
        )
        gateway = connection
        gatewayStatus = viewModelScope.launch {
            connection.status.collect { status ->
                if (generation == requestGeneration) mutable.value = mutable.value.copy(gateway = status)
            }
        }
        connection.start()
    }

    private fun addMessage(message: ChatMessage) {
        val current = mutable.value.messages
        if (current.none { it.id == message.id }) {
            mutable.value = mutable.value.copy(messages = (current + message).sortedBy { it.seq.toLongOrNull() ?: Long.MAX_VALUE })
        }
        confirmPending(message)
    }

    private fun confirmPending(message: ChatMessage) {
        pendingSends.confirm(message)?.confirmed?.invoke()
    }

    private fun resyncChannel(channelId: String) {
        val channel = mutable.value.selectedChannel?.takeIf { it.id == channelId } ?: return
        val requestGeneration = ++generation
        closeChannel(clear = false, clearPending = false)
        mutable.value = mutable.value.copy(gateway = GatewayStatus.CONNECTING, busy = true, error = null)
        viewModelScope.launch {
            try {
                val history = api.history(requireToken(), channel.id)
                if (generation != requestGeneration || mutable.value.selectedChannel?.id != channel.id) return@launch
                history.messages.forEach(::confirmPending)
                mutable.value = mutable.value.copy(messages = history.messages, busy = false)
                openGateway(channel.id, history.cursor, requestGeneration)
            } catch (error: Throwable) {
                if (generation != requestGeneration) return@launch
                if (error is ApiException && error.status in listOf(401, 403, 404)) revokeChannel() else fail(error)
            }
        }
    }

    private fun revokeChannel() {
        ++generation
        closeChannel(clear = true)
        mutable.value = mutable.value.copy(error = "You no longer have access to this channel.")
    }

    private fun closeChannel(clear: Boolean, clearPending: Boolean = true) {
        gateway?.close()
        gateway = null
        gatewayStatus?.cancel()
        gatewayStatus = null
        if (clearPending) pendingSends.clear()
        if (clear) mutable.value = mutable.value.copy(selectedChannel = null, messages = emptyList(), gateway = GatewayStatus.DISCONNECTED)
    }

    private fun invalidate() { ++generation; closeChannel(clear = true) }
    private fun requireToken() = checkNotNull(accountToken) { "Sign in required." }
    private fun setSignedOut() { mutable.value = AppUiState(screen = SessionScreen.SignedOut) }
    private fun fail(error: Throwable) { mutable.value = mutable.value.copy(busy = false, error = error.message ?: "That request did not work.") }
    fun clearError() { mutable.value = mutable.value.copy(error = null) }

    private fun launchBusy(block: suspend (Long) -> Unit) = viewModelScope.launch {
        val requestAccountGeneration = accountGeneration
        mutable.value = mutable.value.copy(busy = true, error = null)
        try { block(requestAccountGeneration) }
        catch (error: Throwable) { if (requestAccountGeneration == accountGeneration) fail(error) }
        finally {
            if (requestAccountGeneration == accountGeneration) mutable.value = mutable.value.copy(busy = false)
        }
    }

    override fun onCleared() { gateway?.close(); super.onCleared() }
}
