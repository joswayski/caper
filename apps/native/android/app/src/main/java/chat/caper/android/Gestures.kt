package chat.caper.android

import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.runtime.Composable
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.ui.Modifier
import androidx.compose.ui.layout.LayoutCoordinates
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.input.pointer.PointerEventPass
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.unit.dp
import kotlin.math.abs

/** One direction decision per touch; vertical and leftward starts never become back swipes. */
internal class ThreadBackSwipe(private val slop: Float, private val distance: Float) {
    var claimed = false
        private set
    private var rejected = false

    fun move(dx: Float, dy: Float, interrupted: Boolean): Boolean {
        if (interrupted) rejected = true
        if (!claimed && !rejected && maxOf(abs(dx), abs(dy)) > slop) {
            if (dx > slop && dx > abs(dy) * 1.5f) claimed = true else rejected = true
        }
        return claimed
    }

    fun released(dx: Float): Boolean = claimed && !rejected && dx >= distance
}

/** Applied only to the phone thread's header/timeline, never its editor or the Browse drawer. */
@Composable internal fun Modifier.threadBackSwipe(
    enabled: Boolean, drag: (Float) -> Unit, settle: (Boolean) -> Unit,
): Modifier {
    val coordinates = remember { mutableStateOf<LayoutCoordinates?>(null) }
    val currentDrag = rememberUpdatedState(drag)
    val currentSettle = rememberUpdatedState(settle)
    return if (!enabled) this else onGloballyPositioned { coordinates.value = it }.pointerInput(Unit) {
        awaitEachGesture {
            val down = awaitFirstDown(requireUnconsumed = false)
            val layout = coordinates.value ?: return@awaitEachGesture
            val start = layout.localToRoot(down.position)
            val swipe = ThreadBackSwipe(viewConfiguration.touchSlop, 72.dp.toPx())
            var completed = false
            try {
                while (true) {
                    // Let scrolling win until direction lock; then cancel child presses before release.
                    val event = awaitPointerEvent(if (swipe.claimed) PointerEventPass.Initial else PointerEventPass.Main)
                    val change = event.changes.firstOrNull { it.id == down.id } ?: break
                    if (!layout.isAttached) break
                    // Use the rendered transform, not the pending offset state: the timeline
                    // moves beneath the pointer, but the dialog's coordinates remain stationary.
                    val delta = layout.localToRoot(change.position) - start
                    val interrupted = event.changes.size != 1 || change.isConsumed
                    val claimed = swipe.move(delta.x, delta.y, interrupted)
                    if (claimed) {
                        event.changes.forEach { it.consume() }
                        currentDrag.value(delta.x.coerceAtLeast(0f))
                    }
                    if (!change.pressed) {
                        completed = swipe.released(delta.x)
                        break
                    }
                    // Keep consuming a recognized swipe even if a second finger cancels it.
                    if (interrupted && !claimed) break
                }
            } finally {
                if (swipe.claimed) currentSettle.value(completed)
            }
        }
    }
}

/**
 * A press that a finger slides away from never acts: once a touch moves past the touch slop
 * without a scroll, drawer, pager or slider claiming it, the presses beneath it (taps, long
 * presses, links, image and file openers) are cancelled, as Android views cancel theirs and as
 * web and Apple do. Compose's own clickables only cancel when a move is consumed, so without
 * this a sideways swipe on an image opened it on release, and a slow one opened the message menu.
 *
 * Put it on a window's root (the app and each dialog or sheet): it watches the Final pass, after
 * every handler below it has had the move, and consumes that single move, which every press
 * detector checks for. It waits for twice the touch slop: a scroller abandons a gesture whose move
 * is consumed before it reaches its own slop, so a slightly diagonal scroll must get there first.
 */
internal fun Modifier.swipeCancelsPresses(): Modifier = pointerInput(Unit) {
    awaitEachGesture {
        val down = awaitFirstDown(requireUnconsumed = false, pass = PointerEventPass.Final)
        val slop = viewConfiguration.touchSlop * 2
        while (true) {
            val event = awaitPointerEvent(PointerEventPass.Final)
            val change = event.changes.firstOrNull { it.id == down.id } ?: break
            // A lift, a second finger (pinches) or a claimed move ends the watch: whoever consumed
            // the move (a list, drawer, slider or long press) has already cancelled the presses.
            if (!change.pressed || event.changes.size != 1 || change.isConsumed) break
            if ((change.position - down.position).getDistance() > slop) {
                change.consume()
                break
            }
        }
    }
}
