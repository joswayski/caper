package chat.caper.android

import android.content.Context
import java.time.Clock
import java.time.LocalDate
import java.time.ZoneOffset
import kotlin.random.Random
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext

internal data class BrandingAvatarAssignment(val utcDay: Long, val avatarIndex: Int)

internal fun brandingAvatarAssignment(
    previous: BrandingAvatarAssignment?,
    utcDay: Long,
    randomIndex: (Int) -> Int,
): BrandingAvatarAssignment {
    val previousIndex = previous?.avatarIndex?.takeIf { it in 0 until BRANDING_AVATAR_COUNT }
    if (previous?.utcDay == utcDay && previousIndex != null) return BrandingAvatarAssignment(utcDay, previousIndex)
    val index = if (previousIndex != null) {
        val candidate = randomIndex(BRANDING_AVATAR_COUNT - 1)
        if (candidate >= previousIndex) candidate + 1 else candidate
    } else {
        randomIndex(BRANDING_AVATAR_COUNT)
    }
    return BrandingAvatarAssignment(utcDay, index)
}

internal class DailyBrandingAvatar(
    private val context: Context,
    private val clock: Clock = Clock.systemUTC(),
    private val randomIndex: (Int) -> Int = Random.Default::nextInt,
) {
    private val preferences = context.getSharedPreferences(PREFERENCES, Context.MODE_PRIVATE)

    /** Rotate only in-app branding; launcher icons always use the original mascot. */
    suspend fun update(): Int = withContext(Dispatchers.IO) {
        updateMutex.withLock {
            val savedIndex = preferences.getInt(KEY_INDEX, -1)
            val savedDay = preferences.getLong(KEY_DAY, Long.MIN_VALUE)
            val previous = savedIndex.takeIf { it in 0 until BRANDING_AVATAR_COUNT }
                ?.let { BrandingAvatarAssignment(savedDay, it) }
            val today = LocalDate.now(clock.withZone(ZoneOffset.UTC)).toEpochDay()
            if (previous?.utcDay == today) return@withLock previous.avatarIndex
            val assignment = brandingAvatarAssignment(previous, today, randomIndex)
            preferences.edit().putLong(KEY_DAY, assignment.utcDay).putInt(KEY_INDEX, assignment.avatarIndex).commit()
            assignment.avatarIndex
        }
    }

    private companion object {
        val updateMutex = Mutex()
        // Retain the old preference keys for the in-app daily character.
        const val PREFERENCES = "launcher_avatar"
        const val KEY_DAY = "utc_day"
        const val KEY_INDEX = "avatar_index"
    }
}

internal const val BRANDING_AVATAR_COUNT = 800
