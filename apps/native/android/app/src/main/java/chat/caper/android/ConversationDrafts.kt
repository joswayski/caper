package chat.caper.android

import chat.caper.android.data.AttachmentEdits
import chat.caper.android.data.VoiceMessages
import chat.caper.android.model.AttachmentDraftTarget
import chat.caper.android.model.ChatAttachment
import chat.caper.android.model.DraftAttachmentUi
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

/** A draft's uploaded staged files as a save sends them: IDs in tile order, and the spoiler-marked ones. */
internal data class DraftFiles(val ids: List<String>, val spoilers: List<String>) {
    companion object {
        /** The composer tiles' files whose upload completed (synced tiles included); files still on their way wait. */
        fun of(tiles: List<DraftAttachmentUi>): DraftFiles {
            val ready = tiles.mapNotNull { tile -> tile.attachment?.let { it.id to tile.spoiler } }
            return DraftFiles(ready.map { it.first }, ready.filter { it.second }.map { it.first })
        }
        fun ofServer(files: List<ChatAttachment>) = DraftFiles(files.map { it.id }, files.filter { it.spoiler }.map { it.id })
    }
}

/** One draft to save to the server ([ConversationDrafts.pendingSave]); null [files] leaves the server's files as they are. */
internal data class DraftSave(val key: String, val channelId: String, val threadRootId: String?, val text: String, val files: DraftFiles? = null)

/** A server draft that applied: whether the text here changed, and the files its composer now shows ([reconcileDraftTiles]). */
internal data class DraftApplied(val key: String, val target: AttachmentDraftTarget, val textChanged: Boolean, val attachments: List<ChatAttachment>)

/** What [ConversationDrafts.merge] did: the server drafts that applied, and dirty drafts to save now. */
internal data class DraftMerge(val applied: List<DraftApplied>, val saves: List<String>)

/** A composer's tiles after a server draft applied, and the local tiles it dropped (to release as Remove does). */
internal data class DraftTilesMerge(val tiles: List<DraftAttachmentUi>, val dropped: List<DraftAttachmentUi>)

/**
 * Rule 4 of docs/media.md "Draft sync": the server's files become the composer's ready tiles, in its order (this
 * device's own uploads keep their tile, other devices' arrive as [syncedDraftTile]s); this device's files still
 * preparing, uploading or failed stay after them; ready local tiles the server no longer lists are dropped.
 */
internal fun reconcileDraftTiles(local: List<DraftAttachmentUi>, server: List<ChatAttachment>, newKey: () -> String): DraftTilesMerge {
    val listed = server.map { it.id }.toSet()
    val ready = local.filter { it.attachment != null }.associateBy { it.attachment!!.id }
    val tiles = server.map { file ->
        val tile = ready[file.id]
        when {
            tile == null -> syncedDraftTile(newKey(), file)
            tile.synced -> tile.copy(attachment = file, spoiler = file.spoiler, thumbnail = syncedThumbnail(file) ?: tile.thumbnail)
            else -> tile.copy(spoiler = file.spoiler)
        }
    } + local.filter { it.attachment == null }
    return DraftTilesMerge(tiles, local.filter { tile -> tile.attachment?.id?.let { it !in listed } == true })
}

/** New signatures for synced tiles a server draft still lists, when it doesn't apply (an echo, or not newer). */
internal fun refreshSyncedTiles(local: List<DraftAttachmentUi>, server: List<ChatAttachment>): List<DraftAttachmentUi> {
    val listed = server.associateBy { it.id }
    return local.map { tile ->
        val attachment = tile.attachment
        val file = attachment?.let { listed[it.id] }
        if (!tile.synced || attachment == null || file == null) tile
        else tile.copy(attachment = attachment.copy(url = file.url, previewUrl = file.previewUrl), thumbnail = syncedThumbnail(file) ?: tile.thumbnail)
    }
}

/** A file another device staged: a ready tile from its signed URLs, with no upload and no local original. */
internal fun syncedDraftTile(key: String, file: ChatAttachment) = DraftAttachmentUi(
    key, if (file.voice) VoiceMessages.LABEL else file.name, file.kind, sourceSize = file.size, storedSize = file.size,
    thumbnail = syncedThumbnail(file),
    // The editor opens it once Edit has downloaded it; animated files (GIFs as video) aren't editable.
    editKind = when {
        file.animated || file.voice -> null
        file.kind == "image" && AttachmentEdits.editableImageType(file.contentType) -> "image"
        file.kind == "video" -> "video"
        else -> null
    },
    compressing = false, progress = 1f, attachment = file, spoiler = file.spoiler,
    voice = file.voice, durationMs = file.durationMs, waveform = file.waveform, synced = true,
)

private fun syncedThumbnail(file: ChatAttachment): String? = file.previewUrl ?: file.url?.takeIf { file.kind == "image" }

/**
 * Unsent composer text of each conversation by channel or DM ID, and thread replies by [thread] key, so switching
 * conversations and coming back restores it. Like web's `localStorage`, it is also kept on this device for the
 * signed-in account (one JSON file each, outside backups), so quitting, a restart or an update restores it too.
 * Signing out deletes the account's file. Text only: staged files, reply targets and the thread's "also send to"
 * choice stay in memory. Drafts are never logged.
 *
 * The text also syncs between the account's devices (docs/media.md "Draft sync"): each local edit marks its draft
 * dirty until a save of that text lands ([pendingSave], [saved]); a server draft applies unless the draft is dirty
 * or the draft is not newer than the last one applied ([apply], [merge]). Its uploaded staged files sync too
 * ([setFiles]); like the files themselves, what this device knows of them is kept in memory only.
 */
internal class ConversationDrafts(private val writer: Executor = Executor { it.run() }) {
    /**
     * A draft's sync state: its channel (needed to save a thread's), the last server `updatedAt` applied, unsaved
     * edits, and its uploaded files as last saved, applied or staged here (null until known since [open], so a save
     * then leaves the server's files alone).
     */
    private data class Sync(val channelId: String? = null, val updatedAt: String? = null, val dirty: Boolean = false, val files: DraftFiles? = null)

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

    /**
     * [target]'s uploaded files changed here (an upload completed, a file was removed or its spoiler mark toggled):
     * an edit like typing. Nothing changes while the files are unknown and none is uploaded, so a composer that
     * hasn't seen the server's files never clears them.
     */
    fun setFiles(target: AttachmentDraftTarget, files: DraftFiles) {
        val key = key(target)
        val state = sync[key] ?: Sync()
        if (state.files == files || state.files == null && files.ids.isEmpty()) return
        sync[key] = state.copy(channelId = target.channelId, dirty = true, files = files)
        save()
        edited(key, false)
    }

    /** Drafts with edits no save has landed yet. */
    fun dirtyKeys(): List<String> = sync.filterValues { it.dirty }.keys.toList()

    /** [key]'s text and known files to save, while it has unsaved edits and its conversation is known. */
    fun pendingSave(key: String): DraftSave? {
        val state = sync[key]?.takeIf { it.dirty } ?: return null
        val channelId = state.channelId ?: return null
        return DraftSave(key, channelId, key.takeIf { it.startsWith(THREAD) }?.removePrefix(THREAD), get(key), state.files)
    }

    /**
     * A save landed: its answer counts as applied (so its own echo changes nothing); edits since stay dirty. A save that
     * left the files alone (not known here) learns them from the answer, returned for the composer to show; its text stays.
     */
    fun saved(save: DraftSave, result: ServerDraft): DraftApplied? {
        val state = sync[save.key] ?: Sync(save.channelId)
        val updatedAt = listOfNotNull(state.updatedAt, result.updatedAt).max()
        val learned = if (save.files == null && state.files == null) result.attachments.filter { it.isValid() } else null
        sync[save.key] = state.copy(updatedAt = updatedAt, dirty = state.dirty && (get(save.key) != save.text || state.files != save.files),
            files = learned?.let { DraftFiles.ofServer(it) } ?: state.files)
        prune()
        save()
        return learned?.let { DraftApplied(save.key, AttachmentDraftTarget(save.channelId, save.threadRootId), false, it) }
    }

    /** The server refused a save for good (no access, too long): the text stays on this device only. */
    fun abandon(save: DraftSave) {
        val state = sync[save.key] ?: return
        if (get(save.key) != save.text || state.files != save.files) return
        sync[save.key] = state.copy(dirty = false)
        save()
    }

    /**
     * A server draft (from `GET /api/drafts` or `draft.updated`). Applies unless the draft is dirty or [draft] is not
     * newer than the last one applied; the same one applies again while this device doesn't know the draft's files
     * (they stay in memory, so after a restart its files come back). Null when it didn't apply; otherwise the composer
     * takes the new text when [DraftApplied.textChanged] and shows its files ([reconcileDraftTiles]).
     */
    fun apply(draft: ServerDraft): DraftApplied? {
        val key = key(draft)
        val state = sync[key]
        if (state?.dirty == true) return null
        if (state?.updatedAt != null && (draft.updatedAt < state.updatedAt || draft.updatedAt == state.updatedAt && state.files != null)) return null
        val files = draft.attachments.filter { it.isValid() }
        sync[key] = Sync(draft.channelId, draft.updatedAt, files = DraftFiles.ofServer(files))
        val changed = write(key, draft.text)
        prune()
        save()
        return DraftApplied(key, AttachmentDraftTarget(draft.channelId, draft.threadRootId), changed, files)
    }

    /**
     * After `GET /api/drafts`: applies each listed draft ([apply]), then marks every non-empty draft the server
     * doesn't list dirty, so drafts typed before sync or offline reach the other devices. [DraftMerge.saves] lists
     * every draft to save now (those, and earlier edits whose save failed).
     */
    fun merge(server: List<ServerDraft>): DraftMerge {
        val applied = server.mapNotNull { apply(it) }
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
        val cleared = sync.filter { (key, state) -> !state.dirty && key !in drafts && state.files?.ids.isNullOrEmpty() }
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

        /** The key of a composer's draft: its thread's, or its conversation's. */
        fun key(target: AttachmentDraftTarget) = target.threadRootId?.let(::thread) ?: target.channelId
    }
}
