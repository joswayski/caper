package chat.caper.android

import chat.caper.android.model.DraftAttachmentUi

/** How long a removed staged file waits before it is really released (`REMOVAL_GRACE` in the spec). */
internal const val REMOVAL_GRACE_MS = 30_000L

/**
 * Staged files removed in the last [REMOVAL_GRACE_MS], hidden from the composer but still
 * uploading (or uploaded), so adding the same picked URI to the same [P] (a conversation or
 * thread composer, or a profile DM draft) restores the draft instead of uploading it again.
 * Only unedited, non-voice files with a picked URI are parked. Progress may update a parked
 * file from any thread, so every method is synchronized.
 */
internal class ParkedDrafts<P> {
    private val parked = LinkedHashMap<String, Pair<P, DraftAttachmentUi>>()

    /** Hides [draft] staged at [place]; false when it can't be parked and is released now instead. */
    @Synchronized fun park(place: P, draft: DraftAttachmentUi): Boolean {
        if (draft.edit != null || draft.voice || draft.pickedUri == null) return false
        parked[draft.key] = place to draft
        return true
    }

    /** The latest file parked at [place] from [source], taken back with its spoiler mark cleared. */
    @Synchronized fun restore(place: P, source: String): DraftAttachmentUi? {
        val key = parked.entries.lastOrNull { (_, entry) -> entry.first == place && entry.second.pickedUri == source }?.key ?: return null
        return parked.remove(key)?.second?.copy(spoiler = false)
    }

    /** Takes out the file with [key] (its grace period ended), to release. */
    @Synchronized fun release(key: String): DraftAttachmentUi? = parked.remove(key)?.second

    /** Takes out every file parked where [where] holds, to release. */
    @Synchronized fun releaseAll(where: (P) -> Boolean = { true }): List<DraftAttachmentUi> {
        val gone = parked.filterValues { where(it.first) }
        parked.keys.removeAll(gone.keys)
        return gone.values.map { it.second }
    }

    @Synchronized operator fun get(key: String): DraftAttachmentUi? = parked[key]?.second

    @Synchronized fun update(key: String, change: (DraftAttachmentUi) -> DraftAttachmentUi) {
        val (place, draft) = parked[key] ?: return
        parked[key] = place to change(draft)
    }

    @Synchronized fun drafts(): List<DraftAttachmentUi> = parked.values.map { it.second }
}
