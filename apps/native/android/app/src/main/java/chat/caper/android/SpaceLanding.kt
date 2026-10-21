package chat.caper.android

import android.content.Context
import android.content.SharedPreferences
import androidx.core.content.edit

/**
 * The space and channel the account last opened, kept across launches like web's
 * `caper:space-landing:<account>`. IDs only, per account and device; direct messages,
 * visits and unjoined previews are never stored.
 */
internal data class SpaceLanding(val spaceId: String, val channelId: String?)

internal object SpaceLandings {
    private const val PREFERENCES = "space-landing"

    private fun preferences(context: Context): SharedPreferences =
        context.getSharedPreferences(PREFERENCES, Context.MODE_PRIVATE)

    fun load(context: Context, accountId: String): SpaceLanding? = load(preferences(context), accountId)

    fun save(context: Context, accountId: String, landing: SpaceLanding) = save(preferences(context), accountId, landing)

    internal fun load(preferences: SharedPreferences, accountId: String): SpaceLanding? {
        if (accountId.isEmpty()) return null
        // Missing or unreadable storage falls back to the first joined channel.
        val space = runCatching { preferences.getString("$accountId.space", null) }.getOrNull()
        if (space.isNullOrEmpty()) return null
        val channel = runCatching { preferences.getString("$accountId.channel", null) }.getOrNull()
        return SpaceLanding(space, channel?.takeIf { it.isNotEmpty() })
    }

    internal fun save(preferences: SharedPreferences, accountId: String, landing: SpaceLanding) {
        if (accountId.isEmpty() || landing.spaceId.isEmpty()) return
        preferences.edit {
            putString("$accountId.space", landing.spaceId)
            if (landing.channelId != null) putString("$accountId.channel", landing.channelId) else remove("$accountId.channel")
        }
    }
}

/** Launch opens the saved landing's space while it is still listed, else the first space. */
internal fun initialSpaceId(spaceIds: List<String>, saved: SpaceLanding?): String? =
    saved?.spaceId?.takeIf { it in spaceIds } ?: spaceIds.firstOrNull()
