package chat.caper.android.data

import chat.caper.android.model.ChatMessage
import chat.caper.android.model.EditUpdate
import chat.caper.android.model.MessageVersions
import com.github.difflib.DiffUtils
import java.time.Instant

internal fun EditUpdate.validated(expectedChannel: String): EditUpdate {
    require(type == "message.edited" && schemaVersion == 1) { "Invalid edit type or schema." }
    require(channelId == expectedChannel && message.channelId == channelId) { "Edit channel mismatch." }
    message.validated(channelId)
    require(message.revision > 1 && message.editSeq == seq) { "Invalid edit revision." }
    return this
}

/**
 * Content, pins, reactions, files and thread summaries have independent revisions. An edit
 * changes text only: files follow `attachmentsSeq`, so an edit snapshot never rolls a
 * "ready" file back to an older "processing" one.
 */
internal fun mergeEdit(current: ChatMessage, incoming: ChatMessage): ChatMessage {
    if (current.id != incoming.id || current.channelId != incoming.channelId || incoming.revision <= current.revision) return current
    val edited = current.copy(
        content = incoming.content.copy(attachments = current.content.attachments),
        revision = incoming.revision, editedAt = incoming.editedAt, editSeq = incoming.editSeq,
    )
    return newerAttachments(edited, incoming)
}

/** Keep loaded overlays; bound only edits awaiting an unloaded page. */
internal fun cacheEditSnapshot(
    snapshots: MutableMap<String, ChatMessage>, incoming: ChatMessage, loadedIds: Set<String>,
): Boolean {
    if (incoming.id !in loadedIds && incoming.id !in snapshots && snapshots.keys.count { it !in loadedIds } >= 256) return false
    snapshots[incoming.id] = snapshots[incoming.id]?.let { mergeEdit(it, incoming) } ?: incoming
    return true
}

internal fun validEditText(text: String) = text.isNotBlank() && text.codePointCount(0, text.length) <= 4000 &&
    text.none { it.isISOControl() && it != '\n' && it != '\t' }

internal fun MessageVersions.validated(messageId: String, before: Int?): MessageVersions {
    require(this.messageId == messageId && versions.size <= 50) { "Invalid message history." }
    versions.forEachIndexed { index, value ->
        require(value.revision > 0 && (before == null || value.revision < before) &&
            (index == 0 || value.revision < versions[index - 1].revision) &&
            value.content.version == 1 && value.content.type == "text" && validEditText(value.content.text) &&
            runCatching { Instant.parse(value.createdAt) }.isSuccess) { "Invalid message version." }
    }
    return this
}

internal data class DiffToken(val text: String, val changed: Boolean)
/** Myers word diff: keep separate edits and preserve every whitespace/Unicode token. */
internal fun messageDiff(before: String, after: String): Pair<List<DiffToken>, List<DiffToken>> {
    val pattern = Regex("\\s+|[\\p{L}\\p{N}_]+|[^\\s\\p{L}\\p{N}_]+")
    val old = pattern.findAll(before).map { it.value }.toList()
    val next = pattern.findAll(after).map { it.value }.toList()
    val removed = mutableSetOf<Int>(); val added = mutableSetOf<Int>()
    DiffUtils.diff(old, next).deltas.forEach { delta ->
        removed.addAll(delta.source.position until delta.source.position + delta.source.lines.size)
        added.addAll(delta.target.position until delta.target.position + delta.target.lines.size)
    }
    return old.mapIndexed { index, text -> DiffToken(text, index in removed) } to next.mapIndexed { index, text -> DiffToken(text, index in added) }
}
