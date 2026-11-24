package chat.caper.android.data

import chat.caper.android.model.ForwardDestination

/**
 * Sharing into Caper from other apps (docs/media.md, "Sharing into Caper"): where a share
 * goes, and the Sharing Shortcut that names it.
 */
data class ShareTarget(val channelId: String, val spaceId: String?, val direct: Boolean) {
    /** `dm:<channel>` or `channel:<space>:<channel>`, the Sharing Shortcut's ID. */
    val shortcutId: String get() = if (direct) "dm:$channelId" else "channel:$spaceId:$channelId"

    companion object {
        private val ID = Regex("^[A-Za-z0-9]{12}$")

        fun of(destination: ForwardDestination): ShareTarget? = when {
            !ID.matches(destination.id) -> null
            destination.direct -> ShareTarget(destination.id, null, true)
            destination.spaceId?.let(ID::matches) == true -> ShareTarget(destination.id, destination.spaceId, false)
            else -> null
        }

        /** The target a Sharing Shortcut names, or null for any other ID. */
        fun fromShortcutId(id: String?): ShareTarget? {
            val parts = id?.split(':') ?: return null
            return when {
                parts.size == 2 && parts[0] == "dm" && ID.matches(parts[1]) -> ShareTarget(parts[1], null, true)
                parts.size == 3 && parts[0] == "channel" && ID.matches(parts[1]) && ID.matches(parts[2]) ->
                    ShareTarget(parts[2], parts[1], false)
                else -> null
            }
        }
    }
}

object Sharing {
    /**
     * Whether a shared stream may be attached: only `content:` URIs another app grants, never
     * `file:` paths or this app's own providers, which could hand over Caper's private files.
     */
    fun acceptsStream(scheme: String?, authority: String?, ownPackage: String): Boolean =
        scheme.equals("content", ignoreCase = true) && authority != null &&
            authority != ownPackage && !authority.startsWith("$ownPackage.")

    /** Destinations whose name or space contains every search word (a leading `#` is ignored), in their order. */
    fun matching(destinations: List<ForwardDestination>, search: String): List<ForwardDestination> {
        val terms = search.trim().split(Regex("\\s+")).map { it.removePrefix("#") }.filter { it.isNotEmpty() }
        return destinations.filter { destination -> terms.all { "${destination.spaceName} ${destination.name}".contains(it, ignoreCase = true) } }
    }

    /** Shared text after what the conversation's draft already holds, on its own line. */
    fun appendText(draft: String, shared: String): String = when {
        shared.isBlank() -> draft
        draft.isBlank() -> shared
        else -> "${draft.trimEnd()}\n$shared"
    }
}
