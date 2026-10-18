package chat.caper.android

import chat.caper.android.data.*
import chat.caper.android.model.*
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update

internal data class ProfileMessageDraft(
    val text: String = "",
    val files: List<DraftAttachmentUi> = emptyList(),
    val sending: Boolean = false,
    val uncertain: Boolean = false,
    val error: String? = null,
) {
    val editable: Boolean get() = !sending && !uncertain
    val canSend: Boolean get() = !sending && (text.isNotBlank() || files.isNotEmpty()) &&
        files.all { it.localUrl != null }
}

/** Profile drafts never use the active conversation's composer or resolve a DM before Send. */
internal class ProfileMessageDrafts(private val api: CaperApi) {
    private val mutable = MutableStateFlow<Map<String, ProfileMessageDraft>>(emptyMap())
    val state = mutable.asStateFlow()
    private val prepared = mutableMapOf<String, PreparedAttachment>()
    private val conversations = mutableMapOf<String, DirectConversation>()
    private val pending = mutableMapOf<String, PendingSendTracker>()
    private val uploader = AttachmentUploader(api)
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

    fun remove(person: String, key: String): Boolean {
        if (!get(person).editable) return false
        prepared.remove(key)
        update(person) { it.copy(files = it.files.filter { file -> file.key != key }, error = null) }
        return true
    }

    /** Logout invalidates in-flight completions as well as every person's text, files and retry ID. */
    fun clear(): List<DraftAttachmentUi> {
        ++epoch
        val files = mutable.value.values.flatMap { it.files }
        mutable.value = emptyMap()
        prepared.clear(); conversations.clear(); pending.clear()
        return files
    }

    suspend fun send(
        person: String, id: String?, username: String, token: String,
        session: suspend () -> Pair<String, ChatAuthor>,
        directs: () -> List<DirectConversation>, blocked: () -> Set<String>,
    ): Pair<DirectConversation, ChatMessage>? {
        val draft = get(person)
        if (!draft.canSend) return null
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
            check(draft.files.all { prepared[it.key] != null }) { "Remove files that could not be prepared first." }
            val (capability, author) = session()
            current()
            val conversation = directs().firstOrNull {
                if (id != null) it.peer.id == id else it.peer.username.equals(username, ignoreCase = true)
            } ?: conversations[person] ?: run {
                // Creating by username can accept a request, so inspect the server's list first.
                val existing = api.directConversations(token).conversations.firstOrNull {
                    if (id != null) it.peer.id == id else it.peer.username.equals(username, ignoreCase = true)
                }
                writable(existing)
                existing ?: api.startDirectConversation(token, username)
            }
            current()
            require(if (id != null) conversation.peer.id == id else conversation.peer.username.equals(username, ignoreCase = true)) { "Direct message recipient mismatch." }
            conversations[person] = conversation
            writable(conversation)
            draft.files.forEach { file ->
                if (file.attachment == null) {
                    writable(conversation)
                    updateFile(person, file.key) { it.copy(error = null, progress = 0f) }
                    try {
                        uploading = true
                        val attachment = uploader.upload(token, conversation.id, prepared.getValue(file.key)) { progress ->
                            if (request == epoch) updateFile(person, file.key) { it.copy(progress = progress) }
                        }
                        current()
                        uploading = false
                        updateFile(person, file.key) { it.copy(attachment = attachment, progress = 1f) }
                    } catch (error: Throwable) {
                        if (error is CancellationException) throw error
                        current()
                        updateFile(person, file.key) { it.copy(error = AttachmentPolicy.uploadErrorMessage(error)) }
                        throw error
                    }
                }
            }
            writable(conversation)
            if (draft.uncertain) check(tracker.retrying(conversation.id, author) != null) { "The conversation changed. The previous send is still unconfirmed." }
            operation = tracker.begin(conversation.id, author, draft.text, get(person).files.map { requireNotNull(it.attachment).id },
                spoilerAttachmentIds = get(person).files.filter { it.spoiler }.map { requireNotNull(it.attachment).id }) {}
            val message = api.sendMessage(token, capability, conversation.id, author, operation.id, operation.text, operation.attachmentIds,
                spoilerAttachmentIds = operation.spoilerAttachmentIds)
            current()
            tracker.confirm(message)
            draft.files.forEach { prepared.remove(it.key) }
            mutable.update { it - person }
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
                error = (error as? ApiException)?.code?.let(::directMessageError)
                    ?: if (uploading) AttachmentPolicy.uploadErrorMessage(error) else friendlyError(error),
            ) }
            return null
        } finally {
            if (request == epoch && person in mutable.value) update(person) { it.copy(sending = false) }
        }
    }

    private fun update(person: String, change: (ProfileMessageDraft) -> ProfileMessageDraft) {
        mutable.update { it + (person to change(it[person] ?: ProfileMessageDraft())) }
    }
}
