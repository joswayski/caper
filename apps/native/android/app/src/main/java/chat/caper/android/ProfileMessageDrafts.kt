package chat.caper.android

import chat.caper.android.data.*
import chat.caper.android.model.*
import java.io.IOException
import java.util.UUID
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock

internal data class ProfileMessageDraft(
    val text: String = "",
    val files: List<DraftAttachmentUi> = emptyList(),
    val sending: Boolean = false,
    val uncertain: Boolean = false,
    /** An unconfirmed standalone GIF; Retry must preserve its exact payload. */
    val pendingGif: KlipyGif? = null,
    val pendingGifQuery: String = "",
    val error: String? = null,
    /** A voice message that didn't send (or isn't confirmed) and waits for Retry or Discard. */
    val voice: ProfileVoiceStatus? = null,
) {
    val editable: Boolean get() = !sending && !uncertain
    val canSend: Boolean get() = !sending && (pendingGif != null || ((text.isNotBlank() || files.isNotEmpty()) &&
        files.all { it.localUrl != null }))
}

/** [uncertain]: a send may have reached the server, so Retry resends the same message. */
internal data class ProfileVoiceStatus(val uncertain: Boolean) {
    val message: String get() =
        if (uncertain) "Voice message delivery isn’t confirmed. Retry resends the same message." else "Voice message didn’t send."
}

/** Profile drafts never use the active conversation's composer; only Send posts a message. */
internal class ProfileMessageDrafts(private val api: CaperApi) {
    private val mutable = MutableStateFlow<Map<String, ProfileMessageDraft>>(emptyMap())
    val state = mutable.asStateFlow()
    private val prepared = mutableMapOf<String, PreparedAttachment>()
    private val conversations = mutableMapOf<String, DirectConversation>()
    private val pending = mutableMapOf<String, PendingSendTracker>()
    private val uploader = AttachmentUploader(api)
    private val uploadLocks = mutableMapOf<String, Mutex>()
    private val voices = mutableMapOf<String, WaitingVoice>()
    @Volatile private var epoch = 0L

    operator fun get(person: String): ProfileMessageDraft = mutable.value[person] ?: ProfileMessageDraft()

    fun text(person: String, text: String) {
        if (!get(person).editable) return
        val limited = if (text.codePointCount(0, text.length) <= 4000) text else text.substring(0, text.offsetByCodePoints(0, 4000))
        update(person) { it.copy(text = limited, error = null) }
    }

    fun error(person: String, error: String) {
        if (get(person).editable) update(person) { it.copy(error = error) }
    }

    fun add(person: String, files: List<DraftAttachmentUi>): List<DraftAttachmentUi> {
        if (!get(person).editable) return emptyList()
        val room = AttachmentPolicy.MAX_ATTACHMENTS - get(person).files.size
        val added = files.take(room)
        update(person) { it.copy(files = it.files + added, error = if (files.size > room) "You can attach up to ${AttachmentPolicy.MAX_ATTACHMENTS} files." else null) }
        return added
    }

    fun updateFile(person: String, key: String, change: (DraftAttachmentUi) -> DraftAttachmentUi) {
        mutable.update { drafts ->
            val draft = drafts[person] ?: return@update drafts
            drafts + (person to draft.copy(files = draft.files.map { file -> if (file.key == key) change(file) else file }))
        }
    }

    fun ready(person: String, key: String, file: PreparedAttachment) {
        if (get(person).files.any { it.key == key }) prepared[key] = file
    }

    /** An edited file prepares again; until then it cannot be sent. */
    fun unprepare(key: String) {
        prepared.remove(key)
    }

    fun remove(person: String, key: String): Boolean {
        if (!get(person).editable) return false
        prepared.remove(key)
        update(person) { it.copy(files = it.files.filter { file -> file.key != key }, error = null) }
        return true
    }

    /**
     * Logout invalidates in-flight completions as well as every person's text, files, retry ID and
     * waiting voice message (returned like a file, so its upload is cancelled and its copy deleted).
     */
    fun clear(): List<DraftAttachmentUi> {
        ++epoch
        val files = mutable.value.values.flatMap { it.files } +
            voices.values.map { DraftAttachmentUi(it.key, VoiceMessages.LABEL, "audio", attachment = it.attachment) }
        mutable.value = emptyMap()
        prepared.clear(); conversations.clear(); pending.clear(); uploadLocks.clear(); voices.clear()
        return files
    }

    /** Selection and Send share the same resolution/upload lock, including concurrent picks. */
    suspend fun upload(
        person: String, id: String?, username: String, token: String,
        directs: () -> List<DirectConversation>, blocked: () -> Set<String>,
        keys: Set<String> = get(person).files.map { it.key }.toSet(),
        request: Long = epoch,
    ): DirectConversation = uploadLocks.getOrPut(person) { Mutex() }.withLock {
        fun writable(conversation: DirectConversation? = null) {
            if (request != epoch) throw CancellationException("Signed out.")
            val latest = directs().firstOrNull { it.id == conversation?.id } ?: conversation
            if (id in blocked() || latest?.blocked == true || latest?.peer?.id in blocked())
                throw ApiException(403, "Blocked", DM_BLOCKED)
            check(latest?.incoming != true) { "Open conversation and accept this message request before replying." }
        }
        writable()
        if (keys.isNotEmpty() && get(person).files.none { it.key in keys }) throw CancellationException("Draft removed.")
        val conversation = directs().firstOrNull {
            if (id != null) it.peer.id == id else it.peer.username.equals(username, ignoreCase = true)
        } ?: conversations[person] ?: run {
            val existing = api.directConversations(token).conversations.firstOrNull {
                if (id != null) it.peer.id == id else it.peer.username.equals(username, ignoreCase = true)
            }
            writable(existing)
            existing ?: api.startDirectConversation(token, username)
        }
        writable(conversation)
        require(if (id != null) conversation.peer.id == id else conversation.peer.username.equals(username, ignoreCase = true)) { "Direct message recipient mismatch." }
        conversations[person] = conversation
        keys.forEach { key ->
            val file = get(person).files.find { it.key == key } ?: return@forEach
            val stale = file.uploadedAt?.let { System.currentTimeMillis() - it >= 23 * 60 * 60 * 1000L } == true
            // Never replace the attachments in an uncertain send's exact retry payload.
            if (file.attachment != null && (!stale || get(person).uncertain)) return@forEach
            val source = prepared[key] ?: error("Remove files that could not be prepared first.")
            writable(conversation)
            updateFile(person, key) { it.copy(attachment = null, error = null, progress = 0f) }
            file.attachment?.let { api.cancelAsset(token, it.id) }
            try {
                val attachment = uploader.upload(token, conversation.id, source) { progress ->
                    if (request == epoch && prepared[key] === source) updateFile(person, key) { it.copy(progress = progress) }
                }
                if (request != epoch || prepared[key] !== source || get(person).files.none { it.key == key }) {
                    api.cancelAsset(token, attachment.id)
                    throw CancellationException("Draft replaced.")
                }
                try { writable(conversation) } catch (error: Throwable) {
                    api.cancelAsset(token, attachment.id)
                    throw error
                }
                updateFile(person, key) { it.copy(attachment = attachment, uploadedAt = System.currentTimeMillis(), progress = 1f) }
            } catch (error: Throwable) {
                if (error is CancellationException) throw error
                if (request == epoch && prepared[key] === source) updateFile(person, key) { it.copy(error = AttachmentPolicy.uploadErrorMessage(error)) }
                throw error
            }
        }
        conversation
    }

    suspend fun send(
        person: String, id: String?, username: String, token: String,
        session: suspend () -> Pair<String, ChatAuthor>,
        directs: () -> List<DirectConversation>, blocked: () -> Set<String>,
        gif: KlipyGif? = null,
        gifQuery: String = "",
    ): Pair<DirectConversation, ChatMessage>? {
        val draft = get(person)
        if ((gif != null && !draft.editable) || (gif == null && !draft.canSend)) return null
        val media = draft.pendingGif ?: gif
        val shareQuery = if (draft.pendingGif != null) draft.pendingGifQuery else gifQuery
        val request = epoch
        fun current() { if (request != epoch) throw CancellationException("Signed out.") }
        fun writable(conversation: DirectConversation? = null) {
            current()
            if (id in blocked() || conversation?.blocked == true || conversation?.peer?.id in blocked())
                throw ApiException(403, "Blocked", DM_BLOCKED)
            check(conversation?.incoming != true) { "Open conversation and accept this message request before replying." }
        }
        val tracker = pending.getOrPut(person) { PendingSendTracker() }
        var operation: PendingSend? = null
        var uploading = false
        update(person) { it.copy(sending = true, error = null) }
        try {
            writable()
            check(media != null || draft.files.all { prepared[it.key] != null }) { "Remove files that could not be prepared first." }
            val (capability, author) = session()
            current()
            uploading = media == null && draft.files.isNotEmpty()
            val conversation = upload(person, id, username, token, directs, blocked, keys = if (media != null) emptySet() else draft.files.map { it.key }.toSet())
            uploading = false
            writable(conversation)
            if (draft.uncertain) check(tracker.retrying(conversation.id, author) != null) { "The conversation changed. The previous send is still unconfirmed." }
            operation = tracker.begin(conversation.id, author, if (media != null) "" else draft.text, if (media != null) emptyList() else get(person).files.map { requireNotNull(it.attachment).id },
                spoilerAttachmentIds = if (media != null) emptyList() else get(person).files.filter { it.spoiler }.map { requireNotNull(it.attachment).id }, gif = media) {}
            val message = api.sendMessage(token, capability, conversation.id, author, operation.id, operation.text, operation.attachmentIds,
                spoilerAttachmentIds = operation.spoilerAttachmentIds, gif = operation.gif)
            current()
            tracker.confirm(message)
            if (media == null) draft.files.forEach { prepared.remove(it.key) }
            // A voice message waiting for Retry stays with the person.
            if (media != null) update(person) { it.copy(sending = false, uncertain = false, pendingGif = null, pendingGifQuery = "", error = null) }
            else mutable.update { drafts -> drafts[person]?.voice?.let { drafts + (person to ProfileMessageDraft(voice = it)) } ?: (drafts - person) }
            pending.remove(person)
            conversations.remove(person)
            return conversation to message
        } catch (error: Throwable) {
            if (error is CancellationException) throw error
            current()
            val definitive = error is ApiException && classifySendFailure(error.status) != SendFailure.UNKNOWN
            if (definitive) operation?.let { tracker.definitiveFailure(it.id) }
            update(person) { it.copy(
                uncertain = if (operation == null) draft.uncertain else !definitive,
                pendingGif = if (operation == null) draft.pendingGif else if (!definitive) media else null,
                pendingGifQuery = if (operation == null) draft.pendingGifQuery else if (!definitive && media != null) shareQuery else "",
                error = (error as? ApiException)?.code?.let(::directMessageError)
                    ?: if (uploading && error !is IllegalStateException) AttachmentPolicy.uploadErrorMessage(error) else friendlyError(error),
            ) }
            return null
        } finally {
            if (request == epoch && person in mutable.value) update(person) { it.copy(sending = false) }
        }
    }

    /**
     * Sends a recorded voice message ([voice], prepared with `voice = true`, its working copy
     * under [key]) to this person's DM on its own, through the same voice upload as the
     * conversation composers. The typed text, staged files and any unconfirmed text send stay as
     * they are; the DM is resolved and checked exactly as for Send. One waits at a time: a send
     * that fails or whose outcome is unknown keeps the recording for [retryVoice] or [discardVoice].
     */
    suspend fun sendVoice(
        person: String, id: String?, username: String, token: String, key: String, voice: PreparedAttachment,
        session: suspend () -> Pair<String, ChatAuthor>,
        directs: () -> List<DirectConversation>, blocked: () -> Set<String>,
    ): Pair<DirectConversation, ChatMessage>? {
        if (!voice.voice || person in voices || !get(person).editable) return null
        voices[person] = WaitingVoice(key, voice)
        return deliverVoice(person, id, username, token, session, directs, blocked)
    }

    /** Sends the waiting voice message again: the same upload, and the same message once a send may have arrived. */
    suspend fun retryVoice(
        person: String, id: String?, username: String, token: String,
        session: suspend () -> Pair<String, ChatAuthor>,
        directs: () -> List<DirectConversation>, blocked: () -> Set<String>,
    ): Pair<DirectConversation, ChatMessage>? {
        if (person !in voices || get(person).sending) return null
        return deliverVoice(person, id, username, token, session, directs, blocked)
    }

    /** The working-copy key of [person]'s waiting voice message, if one waits. */
    fun voiceKey(person: String): String? = voices[person]?.key

    /** Drops the waiting voice message: its reservation is cancelled; returns its key so the caller deletes the file. */
    suspend fun discardVoice(person: String, token: String?): String? {
        if (get(person).sending) return null
        val waiting = voices.remove(person) ?: return null
        update(person) { it.copy(voice = null) }
        dropIfEmpty(person)
        waiting.attachment?.let { if (token != null) api.cancelAsset(token, it.id) }
        return waiting.key
    }

    private suspend fun deliverVoice(
        person: String, id: String?, username: String, token: String,
        session: suspend () -> Pair<String, ChatAuthor>,
        directs: () -> List<DirectConversation>, blocked: () -> Set<String>,
    ): Pair<DirectConversation, ChatMessage>? {
        val waiting = voices[person] ?: return null
        val request = epoch
        fun current() { if (request != epoch) throw CancellationException("Signed out.") }
        update(person) { it.copy(sending = true, error = null) }
        try {
            val (capability, author) = session()
            current()
            val conversation = upload(person, id, username, token, directs, blocked, keys = emptySet(), request = request)
            // Once a send may have arrived, a retry repeats it exactly: same conversation, file and ID.
            if (waiting.clientMessageId != null) check(waiting.conversationId == conversation.id) {
                "The conversation changed. The previous voice message is still unconfirmed."
            }
            val stale = waiting.clientMessageId == null && System.currentTimeMillis() - waiting.uploadedAt >= 23 * 60 * 60 * 1000L
            if (waiting.attachment == null || stale) {
                waiting.attachment?.let { api.cancelAsset(token, it.id) }
                waiting.attachment = null
                waiting.attachment = uploader.upload(token, conversation.id, waiting.file)
                waiting.uploadedAt = System.currentTimeMillis()
                current()
            }
            val attachment = requireNotNull(waiting.attachment)
            val clientMessageId = waiting.clientMessageId ?: UUID.randomUUID().also {
                waiting.clientMessageId = it
                waiting.conversationId = conversation.id
            }
            val send = suspend { api.sendMessage(token, capability, conversation.id, author, clientMessageId, "", listOf(attachment.id)) }
            // Retried once after a lost response, like the composers' sends.
            val message = try { send() } catch (error: IOException) {
                if (error is ApiException) throw error
                delay(250)
                send()
            }
            current()
            voices.remove(person)
            update(person) { it.copy(voice = null) }
            dropIfEmpty(person)
            return conversation to message
        } catch (error: Throwable) {
            if (error is CancellationException) throw error
            current()
            val refused = error is ApiException && classifySendFailure(error.status) != SendFailure.UNKNOWN
            if (refused) {
                // A definitive refusal drops the recording and says why; nothing was or will be sent.
                voices.remove(person)
                waiting.attachment?.let { api.cancelAsset(token, it.id) }
                update(person) { it.copy(voice = null, error = (error as ApiException).code?.let(::directMessageError) ?: friendlyError(error)) }
            } else {
                update(person) { it.copy(voice = ProfileVoiceStatus(uncertain = waiting.clientMessageId != null)) }
            }
            return null
        } finally {
            if (request == epoch && person in mutable.value) update(person) { it.copy(sending = false) }
        }
    }

    /** A draft with nothing left in it (no text, files, error or waiting voice message) goes away. */
    private fun dropIfEmpty(person: String) {
        mutable.update { drafts ->
            val draft = drafts[person] ?: return@update drafts
            if (draft.text.isEmpty() && draft.files.isEmpty() && !draft.uncertain && draft.error == null && draft.voice == null) drafts - person
            else drafts
        }
    }

    /** A recording kept after a failed send: its file, upload and (once a send may have arrived) message ID. */
    private class WaitingVoice(val key: String, val file: PreparedAttachment) {
        var attachment: ChatAttachment? = null
        var uploadedAt = 0L
        var clientMessageId: UUID? = null
        var conversationId: String? = null
    }

    private fun update(person: String, change: (ProfileMessageDraft) -> ProfileMessageDraft) {
        mutable.update { it + (person to change(it[person] ?: ProfileMessageDraft())) }
    }
}
