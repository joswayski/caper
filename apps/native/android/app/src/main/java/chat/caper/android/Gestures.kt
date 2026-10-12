package chat.caper.android

import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.ui.Modifier
import androidx.compose.ui.input.pointer.PointerEventPass
import androidx.compose.ui.input.pointer.pointerInput

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
