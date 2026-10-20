package chat.caper.android

import android.provider.Settings
import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.CubicBezierEasing
import androidx.compose.animation.core.LinearEasing
import androidx.compose.animation.core.tween
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.setValue
import androidx.compose.foundation.lazy.LazyListState
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.layout.onPlaced
import androidx.compose.ui.layout.positionInRoot
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import kotlin.math.roundToInt

// shared/design.css motion tokens (web's motion.css uses the same values).
internal val EaseOut = CubicBezierEasing(0.16f, 1f, 0.3f, 1f)
internal const val MOTION_FAST = 140
internal const val MOTION_BASE = 200
internal const val MOTION_SLOW = 260

/** Drawing-only FLIP compensation: FollowLatest still pins immediately, without changing layout. */
internal class TimelineDisplacement {
    var revision by mutableIntStateOf(0)
        private set
    var remaining by mutableFloatStateOf(0f)
    var viewportY by mutableFloatStateOf(0f)
    private var origins: Map<Any, Float> = emptyMap()

    fun offset(list: LazyListState, key: Any): Float {
        if (remaining == 0f) return 0f
        val visible = list.layoutInfo.visibleItemsInfo
        // An incoming row travels with its preceding rows instead of fading
        // underneath them while they are still making room.
        val row = visible.firstOrNull { it.key == key && origins.containsKey(key) }
            ?: visible.lastOrNull { origins.containsKey(it.key) } ?: return 0f
        val origin = origins.getValue(row.key)
        return (origin - row.offset - viewportY) * remaining
    }

    fun capture(list: LazyListState) {
        origins = list.layoutInfo.visibleItemsInfo.associate { it.key to it.offset + viewportY + offset(list, it.key) }
        remaining = 1f
        revision++
    }

    fun clear() {
        if (remaining == 0f) return
        remaining = 0f
        revision++ // Cancel the frame coroutine too; it must not restore the offset.
    }
}

internal fun Modifier.displacement(motion: TimelineDisplacement, list: LazyListState, key: Any): Modifier =
    graphicsLayer { translationY = motion.offset(list, key) }

// Short bottom-aligned lists can move as a whole; overflowing lists move inside their viewport.
internal fun Modifier.displacementViewport(motion: TimelineDisplacement): Modifier =
    onPlaced { motion.viewportY = it.positionInRoot().y }

/** True when the system's animator duration scale is 0 (Remove animations), web's `prefers-reduced-motion`. */
@Composable internal fun reducedMotion(): Boolean {
    val context = LocalContext.current
    val initial = remember(context) { Settings.Global.getFloat(context.contentResolver, Settings.Global.ANIMATOR_DURATION_SCALE, 1f) == 0f }
    val still by produceState(initial, context) {
        val resolver = context.contentResolver
        val setting = Settings.Global.ANIMATOR_DURATION_SCALE
        fun disabled() = Settings.Global.getFloat(resolver, setting, 1f) == 0f
        val observer = object : android.database.ContentObserver(android.os.Handler(android.os.Looper.getMainLooper())) {
            override fun onChange(selfChange: Boolean) { value = disabled() }
        }
        resolver.registerContentObserver(Settings.Global.getUriFor(setting), false, observer)
        value = disabled()
        awaitDispose { resolver.unregisterContentObserver(observer) }
    }
    return still
}

/**
 * Fades this in once, on first composition, from [dx]/[dy] away and [scale], easing out over
 * [durationMillis]. [elapsedMillis] resumes an entrance already under way (a pending send's row
 * replaced by its confirmed one). Only drawing changes, never layout; nothing animates on the way out.
 */
@Composable internal fun Modifier.entrance(durationMillis: Int, dx: Dp = 0.dp, dy: Dp = 0.dp, scale: Float = 1f, elapsedMillis: Long = 0): Modifier {
    val still = reducedMotion()
    val progress = remember { Animatable(if (still) 1f else (elapsedMillis.toFloat() / durationMillis).coerceIn(0f, 1f)) }
    val animates = remember { progress.value < 1f }
    if (!animates) return this
    LaunchedEffect(progress, still) {
        if (still) progress.snapTo(1f)
        else progress.animateTo(1f, tween(((1f - progress.value) * durationMillis).roundToInt(), easing = LinearEasing))
    }
    return graphicsLayer {
        val eased = if (still) 1f else EaseOut.transform(progress.value)
        alpha = eased
        translationX = dx.toPx() * (1f - eased)
        translationY = dy.toPx() * (1f - eased)
        if (scale != 1f) { scaleX = scale + (1f - scale) * eased; scaleY = scaleX }
    }
}

/** A message that [arrivals] saw arrive at the live end settles 6dp up into place (web's `.chat-message-arrive`). */
@Composable internal fun Modifier.arrival(arrivals: Arrivals, clientMessageId: String): Modifier {
    val elapsed = remember(clientMessageId) { arrivals.elapsed(clientMessageId) } ?: return this
    return entrance(MOTION_SLOW, dy = 6.dp, elapsedMillis = elapsed)
}
