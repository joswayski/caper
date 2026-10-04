package chat.caper.android

import android.content.ComponentName
import android.content.Context
import android.content.pm.PackageManager
import android.os.Build
import java.time.Clock
import java.time.LocalDate
import java.time.ZoneOffset
import kotlin.random.Random
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext

internal data class LauncherAvatarAssignment(val utcDay: Long, val avatarIndex: Int)

internal fun launcherAvatarAssignment(
    previous: LauncherAvatarAssignment?,
    utcDay: Long,
    randomIndex: (Int) -> Int,
): LauncherAvatarAssignment {
    val previousIndex = previous?.avatarIndex?.takeIf { it in 0 until LAUNCHER_AVATAR_COUNT }
    if (previous?.utcDay == utcDay && previousIndex != null) return LauncherAvatarAssignment(utcDay, previousIndex)
    val index = if (previousIndex != null) {
        val candidate = randomIndex(LAUNCHER_AVATAR_COUNT - 1)
        if (candidate >= previousIndex) candidate + 1 else candidate
    } else {
        randomIndex(LAUNCHER_AVATAR_COUNT)
    }
    return LauncherAvatarAssignment(utcDay, index)
}

internal class LauncherAvatarRotator(
    private val context: Context,
    private val clock: Clock = Clock.systemUTC(),
    private val randomIndex: (Int) -> Int = Random.Default::nextInt,
) {
    private val preferences = context.getSharedPreferences(PREFERENCES, Context.MODE_PRIVATE)

    /** Return the applied choice so in-app branding does not draw a second avatar. */
    suspend fun update(): Int = withContext(Dispatchers.IO) {
        updateMutex.withLock {
            val savedIndex = preferences.getInt(KEY_INDEX, -1)
            val savedDay = preferences.getLong(KEY_DAY, Long.MIN_VALUE)
            val previous = savedIndex.takeIf { it in 0 until LAUNCHER_AVATAR_COUNT }
                ?.let { LauncherAvatarAssignment(savedDay, it) }
            val today = LocalDate.now(clock.withZone(ZoneOffset.UTC)).toEpochDay()
            if (previous?.utcDay == today) return@withLock previous.avatarIndex
            val assignment = launcherAvatarAssignment(previous, today, randomIndex)
            if (applyAlias(assignment.avatarIndex)) {
                preferences.edit().putLong(KEY_DAY, assignment.utcDay).putInt(KEY_INDEX, assignment.avatarIndex).commit()
                assignment.avatarIndex
            } else previous?.avatarIndex ?: 0
        }
    }

    private fun applyAlias(index: Int): Boolean = try {
        val manager = context.packageManager
        val desired = avatarComponent(index)
        val enabled = PackageManager.COMPONENT_ENABLED_STATE_ENABLED
        val disabled = PackageManager.COMPONENT_ENABLED_STATE_DISABLED
        val flags = PackageManager.DONT_KILL_APP
        val all = sequenceOf(defaultComponent()) + (0 until LAUNCHER_AVATAR_COUNT).asSequence().map(::avatarComponent)
        // Reconcile effective state after an interrupted transition, not just saved preferences.
        val active = all.filter { component ->
            val state = manager.getComponentEnabledSetting(component)
            state == enabled || state == PackageManager.COMPONENT_ENABLED_STATE_DEFAULT && component == defaultComponent()
        }.toList()
        if (Build.VERSION.SDK_INT >= 33) {
            manager.setComponentEnabledSettings((active + desired).distinct().map { component ->
                PackageManager.ComponentEnabledSetting(component, if (component == desired) enabled else disabled, flags)
            })
        } else {
            // Keep a launcher entry throughout the non-atomic transition on Android 12 and older.
            manager.setComponentEnabledSetting(desired, enabled, flags)
            active.filter { it != desired }
                .forEach { manager.setComponentEnabledSetting(it, disabled, flags) }
        }
        true
    } catch (_: RuntimeException) {
        false
    }

    // Component class names use the namespace, not the .debug application ID suffix.
    private fun defaultComponent() = ComponentName(context, "chat.caper.android.launcher.Default")
    private fun avatarComponent(index: Int) = ComponentName(context, "chat.caper.android.launcher.Avatar$index")

    private companion object {
        val updateMutex = Mutex()
        const val PREFERENCES = "launcher_avatar"
        const val KEY_DAY = "utc_day"
        const val KEY_INDEX = "avatar_index"
    }
}

internal const val LAUNCHER_AVATAR_COUNT = 800
