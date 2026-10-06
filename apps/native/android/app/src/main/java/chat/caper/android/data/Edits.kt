package chat.caper.android.data

import chat.caper.android.model.ChatMessage
import chat.caper.android.model.EditUpdate

internal fun EditUpdate.validated(expectedChannel: String): EditUpdate {
    require(type == "message.edited" && schemaVersion == 1) { "Invalid edit type or schema." }
    require(channelId == expectedChannel && message.channelId == channelId) { "Edit channel mismatch." }
    message.validated(channelId)
    require(message.revision > 1 && message.editSeq == seq) { "Invalid edit revision." }
    return this
}

/** Content, pins, reactions and thread summaries have independent revisions. */
internal fun mergeEdit(current: ChatMessage, incoming: ChatMessage): ChatMessage {
    if (current.id != incoming.id || current.channelId != incoming.channelId || incoming.revision <= current.revision) return current
    return current.copy(content = incoming.content, revision = incoming.revision, editedAt = incoming.editedAt, editSeq = incoming.editSeq)
}

/** Keep loaded overlays; bound only edits awaiting an unloaded page. */
internal fun cacheEditSnapshot(
    snapshots: MutableMap<String, ChatMessage>, incoming: ChatMessage, loadedIds: Set<String>,
): Boolean {
    if (incoming.id !in loadedIds && incoming.id !in snapshots && snapshots.keys.count { it !in loadedIds } >= 256) return false
    snapshots[incoming.id] = snapshots[incoming.id]?.let { mergeEdit(it, incoming) } ?: incoming
    return true
}
