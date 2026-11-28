package chat.caper.android

import chat.caper.android.model.ServerDraft
import java.io.File
import java.util.concurrent.Executor
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.booleanOrNull
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.put
import kotlinx.serialization.json.putJsonObject

/** One draft's text to save to the server ([ConversationDrafts.pendingSave]). */
internal data class DraftSave(val key: String, val channelId: String, val threadRootId: String?, val text: String)

/** What [ConversationDrafts.merge] did: drafts whose text changed, and dirty drafts to save now. */
internal data class DraftMerge(val applied: Set<String>, val saves: List<String>)

/**
 * Unsent composer text of each conversation by channel or DM ID, and thread replies by [thread] key, so switching
 * conversations and coming back restores it. Like web's `localStorage`, it is also kept on this device for the
 * signed-in account (one JSON file each, outside backups), so quitting, a restart or an update restores it too.
 * Signing out deletes the account's file. Text only: staged files, reply targets and the thread's "also send to"
 * choice stay in memory. Drafts are never logged.
 *
 * The text also syncs between the account's devices (docs/media.md "Draft sync"): each local edit marks its draft
 * dirty until a save of that text lands ([pendingSave], [saved]); a server draft applies unless the draft is dirty
 * or the draft is not newer than the last one applied ([apply], [merge]).
 */
internal class ConversationDrafts(private val writer: Executor = Executor { it.run() }) {
    /** A draft's sync state: its channel (needed to save a thread's), the last server `updatedAt` applied, unsaved edits. */
    private data class Sync(val channelId: String? = null, val updatedAt: String? = null, val dirty: Boolean = false)

    private val drafts = mutableMapOf<String, String>()
    private val sync = mutableMapOf<String, Sync>()
    private var account = ""
    private var file: File? = null

    /** Called after each local edit with the draft's key; `now` when the text was cleared (a send), so it saves at once. */
    var edited: (key: String, now: Boolean) -> Unit = { _, _ -> }

    /** Opens [accountId]'s saved drafts in [dir] unless they are the ones open. A missing or unreadable file means none. */
    fun open(dir: File?, accountId: String) {
        if (accountId == account) return
        account = accountId
        drafts.clear()
        sync.clear()
        file = dir?.takeIf { accountId.isNotEmpty() && accountId.all { it.isLetterOrDigit() || it == '-' || it == '_' } }
            ?.let { File(it, "$accountId.json") }
        val saved = runCatching { file?.takeIf { it.exists() }?.readText()?.let { Json.parseToJsonElement(it).jsonObject } }.getOrNull()
        saved?.forEach { (key, value) ->
            if (key == SYNC) return@forEach
            runCatching { value.jsonPrimitive.contentOrNull }.getOrNull()?.takeIf { it.isNotBlank() }?.let { drafts[key] = it }
        }
        runCatching { saved?.get(SYNC)?.jsonObject }.getOrNull()?.forEach { (key, value) ->
            runCatching {
                val entry = value.jsonObject
                sync[key] = Sync(entry["channelId"]?.jsonPrimitive?.contentOrNull, entry["updatedAt"]?.jsonPrimitive?.contentOrNull,
                    entry["dirty"]?.jsonPrimitive?.booleanOrNull == true)
            }
        }
    }

    operator fun get(conversation: String): String = drafts[conversation].orEmpty()

    /** A local edit of a conversation's draft (or a [thread] key whose channel is already known). */
    operator fun set(conversation: String, text: String) = edit(conversation, null, text)

    /** A local edit of [channelId]'s draft, or of its thread [threadRootId]'s reply. */
    operator fun set(channelId: String, threadRootId: String?, text: String) =
        edit(threadRootId?.let(::thread) ?: channelId, channelId, text)

    private fun edit(key: String, channelId: String?, text: String) {
        if (!write(key, text)) return
        val state = sync[key] ?: Sync()
        // A thread reply whose channel isn't known can't be saved, so it stays on this device.
        val channel = channelId ?: state.channelId ?: key.takeUnless { it.startsWith(THREAD) }
        sync[key] = state.copy(channelId = channel, dirty = channel != null)
        save()
        edited(key, text.isBlank())
    }

    /** Drafts with edits no save has landed yet. */
    fun dirtyKeys(): List<String> = sync.filterValues { it.dirty }.keys.toList()

    /** [key]'s text to save, while it has unsaved edits and its conversation is known. */
    fun pendingSave(key: String): DraftSave? {
        val state = sync[key]?.takeIf { it.dirty } ?: return null
        val channelId = state.channelId ?: return null
        return DraftSave(key, channelId, key.takeIf { it.startsWith(THREAD) }?.removePrefix(THREAD), get(key))
    }

    /** A save landed: its answer counts as applied (so its own echo changes nothing); edits since stay dirty. */
    fun saved(save: DraftSave, result: ServerDraft) {
        val state = sync[save.key] ?: Sync(save.channelId)
        val updatedAt = listOfNotNull(state.updatedAt, result.updatedAt).max()
        sync[save.key] = state.copy(updatedAt = updatedAt, dirty = state.dirty && get(save.key) != save.text)
        prune()
        save()
    }

    /** The server refused a save for good (no access, too long): the text stays on this device only. */
    fun abandon(save: DraftSave) {
        val state = sync[save.key] ?: return
        if (get(save.key) != save.text) return
        sync[save.key] = state.copy(dirty = false)
        save()
    }

    /**
     * A server draft (from `GET /api/drafts` or `draft.updated`). Applies unless the draft is dirty or [draft] is not
     * newer than the last one applied. True when the text here changed, so a composer showing it takes the new text.
     */
    fun apply(draft: ServerDraft): Boolean {
        val key = key(draft)
        val state = sync[key]
        if (state?.dirty == true || state?.updatedAt != null && draft.updatedAt <= state.updatedAt) return false
        sync[key] = Sync(draft.channelId, draft.updatedAt)
        val changed = write(key, draft.text)
        prune()
        save()
        return changed
    }

    /**
     * After `GET /api/drafts`: applies each listed draft ([apply]), then marks every non-empty draft the server
     * doesn't list dirty, so drafts typed before sync or offline reach the other devices. [DraftMerge.saves] lists
     * every draft to save now (those, and earlier edits whose save failed).
     */
    fun merge(server: List<ServerDraft>): DraftMerge {
        val applied = server.filter { apply(it) }.map { key(it) }.toSet()
        val listed = server.map { key(it) }.toSet()
        drafts.keys.filter { it !in listed }.forEach { key ->
            val state = sync[key] ?: Sync(key.takeUnless { it.startsWith(THREAD) })
            // A thread reply saved before sync knew its channel stays on this device.
            if (state.channelId != null) sync[key] = state.copy(dirty = true)
        }
        save()
        return DraftMerge(applied, dirtyKeys().filter { pendingSave(it) != null })
    }

    /** Signing out: every draft goes, and the account's file with them, so another account never sees them. */
    fun clear() {
        drafts.clear()
        sync.clear()
        account = ""
        val gone = file
        file = null
        if (gone != null) writer.execute { gone.delete() }
    }

    /** Sets the device copy; true when it changed. Blank text removes it. */
    private fun write(key: String, text: String): Boolean =
        if (text.isBlank()) drafts.remove(key) != null else drafts.put(key, text) != text

    /**
     * Cleared, saved drafts keep their last `updatedAt`, so an older copy still on its way (a reload that started
     * before a send) can't bring sent text back; only the newest [MAX_CLEARED] are kept.
     */
    private fun prune() {
        val cleared = sync.filter { (key, state) -> !state.dirty && key !in drafts }
        if (cleared.size <= MAX_CLEARED) return
        cleared.entries.sortedBy { it.value.updatedAt.orEmpty() }.take(cleared.size - MAX_CLEARED).forEach { sync.remove(it.key) }
    }

    private fun save() {
        val target = file ?: return
        val snapshot = buildJsonObject {
            drafts.forEach { (key, text) -> put(key, text) }
            if (sync.isNotEmpty()) putJsonObject(SYNC) {
                sync.forEach { (key, state) ->
                    putJsonObject(key) {
                        state.channelId?.let { put("channelId", it) }
                        state.updatedAt?.let { put("updatedAt", it) }
                        if (state.dirty) put("dirty", true)
                    }
                }
            }
        }.toString()
        val empty = drafts.isEmpty() && sync.isEmpty()
        writer.execute {
            // Write beside it, then rename: a crash never leaves half a file. Never log the drafts or their path.
            runCatching {
                if (empty) target.delete()
                else {
                    target.parentFile?.mkdirs()
                    val temporary = File(target.path + ".tmp")
                    temporary.writeText(snapshot)
                    if (!temporary.renameTo(target)) { target.delete(); temporary.renameTo(target) }
                }
            }
        }
    }

    companion object {
        private const val THREAD = "thread:"
        /** The file's sync state; never a channel ID or [thread] key. */
        private const val SYNC = "~sync"
        private const val MAX_CLEARED = 500

        /** A thread reply's key: root message IDs never collide with channel IDs. */
        fun thread(rootId: String) = "$THREAD$rootId"

        /** The key a server draft is kept under here. */
        fun key(draft: ServerDraft) = draft.threadRootId?.let(::thread) ?: draft.channelId
    }
}
