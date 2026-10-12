package chat.caper.android

import android.provider.Settings
import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.CubicBezierEasing
import androidx.compose.animation.core.LinearEasing
import androidx.compose.animation.core.tween
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import kotlin.math.roundToInt

// shared/design.css motion tokens (web's motion.css uses the same values).
internal val EaseOut = CubicBezierEasing(0.16f, 1f, 0.3f, 1f)
internal const val MOTION_FAST = 140
internal const val MOTION_BASE = 200
internal const val MOTION_SLOW = 260

/** True when the system's animator duration scale is 0 (Remove animations), web's `prefers-reduced-motion`. */
@Composable internal fun reducedMotion(): Boolean {
    val context = LocalContext.current
    return remember { Settings.Global.getFloat(context.contentResolver, Settings.Global.ANIMATOR_DURATION_SCALE, 1f) == 0f }
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
    LaunchedEffect(progress) { progress.animateTo(1f, tween(((1f - progress.value) * durationMillis).roundToInt(), easing = LinearEasing)) }
    return graphicsLayer {
        val eased = EaseOut.transform(progress.value)
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
