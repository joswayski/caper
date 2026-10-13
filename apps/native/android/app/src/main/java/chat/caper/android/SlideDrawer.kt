package chat.caper.android

import androidx.compose.animation.core.CubicBezierEasing
import androidx.compose.animation.core.animate
import androidx.compose.animation.core.tween
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.Stable
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.input.pointer.PointerEventPass
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.input.pointer.util.VelocityTracker
import androidx.compose.ui.input.pointer.util.addPointerInputChange
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.dp
import kotlin.math.abs
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.launch

/**
 * Web `TAP` easing, for slides a button, Back or a channel choice starts: it leaves at once and
 * slows steadily, rather than leaping most of the way in two frames and then crawling into place.
 */
private val TapEasing = CubicBezierEasing(0.33f, 1f, 0.68f, 1f)

/** A released swipe's curve: it starts at the finger's speed and lands at rest without overshooting. */
private fun releaseEasing(slope: Float) = CubicBezierEasing(0.2f, 0.2f * slope, 0.35f, 1f)

/**
 * Phones show the conversation or Browse (spaces, channels and the account bar), never both, and
 * slide the member list over the conversation. Each slide is one of these: a finger moves it 1:1,
 * a release carries on at the finger's speed, and buttons, Back and channel choices animate the
 * same slide. A finger that lands on a moving slide grabs it where it is.
 *
 * The position is plain state read only in layer blocks, so a moving finger redraws the layers
 * without recomposing the conversation. The one animation runs as a [Job] that a drag cancels
 * synchronously, so a slide never fights the finger or jumps back to where the drag began.
 * System animator duration scale (including Remove animations) applies through Compose's
 * frame clock: at 0 every slide lands at once.
 */
@Stable
internal class SlideDrawer(private val scope: CoroutineScope, open: Boolean) {
    private val position = mutableFloatStateOf(if (open) 1f else 0f)
    private val destination = mutableFloatStateOf(if (open) 1f else 0f)
    private var motion: Job? = null

    /** How far open it is, from 0 (closed: the conversation, or no member list) to 1. */
    val value: Float get() = position.floatValue
    /** Where it rests or is sliding to, 0 or 1. A finger's drag leaves it unchanged until release. */
    val target: Float get() = destination.floatValue
    var dragging = false
    /** Set when a touch lands on content or its header; editors and audio controls keep their own drags. */
    var regionTouched = false
    /** Set when a touch lands on an editor inside a region, which keeps its own drags. */
    var regionExcluded = false

    /** Follows a finger: stops any slide and holds the drawer at [to]. */
    fun drag(to: Float) {
        motion?.cancel()
        motion = null
        position.floatValue = to.coerceIn(0f, 1f)
    }

    /** Slides open or closed: a button's [DRAWER_TAP_MILLIS] curve, or a released swipe's [release] timing. */
    fun settle(open: Boolean, release: ReleaseTiming? = null) {
        val end = if (open) 1f else 0f
        if (destination.floatValue == end && (motion?.isActive == true || position.floatValue == end)) return
        motion?.cancel()
        destination.floatValue = end
        val from = position.floatValue
        if (from == end) { motion = null; return }
        val spec = if (release != null) tween<Float>(release.durationMillis, easing = releaseEasing(release.slope))
            else tween<Float>(drawerTapMillis(end - from), easing = TapEasing)
        motion = scope.launch { animate(from, end, animationSpec = spec) { value, _ -> position.floatValue = value } }
    }
}

@Composable internal fun rememberSlideDrawer(open: Boolean): SlideDrawer {
    val scope = rememberCoroutineScope()
    return remember(scope) { SlideDrawer(scope, open) }
}

internal val LocalBrowseDrawer = staticCompositionLocalOf<SlideDrawer?> { null }
/** The phone member list's drawer: 0 hidden, 1 shown. */
internal val LocalMembersDrawer = staticCompositionLocalOf<SlideDrawer?> { null }

/** The phone member list's inset from the trailing edge. */
internal val MEMBERS_END = 8.dp
/** How far past its width the member list travels: its inset, plus room for its shadow. */
internal val MEMBERS_MARGIN = MEMBERS_END + 24.dp
/** The phone member list's widest. */
internal val MEMBERS_MAX_WIDTH = 280.dp

/** How far past the screen's edge the conversation travels, so its shadow leaves with it (web and Apple). */
internal val BROWSE_SHADOW = 48.dp

/** Browse's travel for the container's width: off the edge, shadow and all. */
internal fun Density.browseTravel(containerWidth: Float): Float = containerWidth + BROWSE_SHADOW.toPx()

/**
 * Moves the conversation, a layer above the whole of Browse (account bar included), fully over
 * or off it at its full height. Browse stays still beneath it.
 */
internal fun Modifier.conversationLayer(drawer: SlideDrawer): Modifier = graphicsLayer {
    val uncovered = drawer.value
    translationX = uncovered * browseTravel(size.width)
    shadowElevation = if (uncovered > 0f) 16.dp.toPx() else 0f
}

/**
 * Slides the phone member list in from the trailing edge, outlined and lifted
 * by the same shadow the conversation casts over Browse.
 */
internal fun Modifier.membersLayer(drawer: SlideDrawer): Modifier = graphicsLayer {
    translationX = (1f - drawer.value) * (size.width + MEMBERS_MARGIN.toPx())
    shape = RoundedCornerShape(16.dp)
    clip = true
    shadowElevation = 16.dp.toPx()
}

// Content, headers and drawer backdrops start this gesture; the composer and audio controls
// keep their native drags.
internal fun Modifier.browseSwipeRegion(drawer: SlideDrawer?, enabled: Boolean): Modifier =
    if (drawer == null || !enabled) this else pointerInput(drawer) {
        awaitEachGesture {
            awaitFirstDown(requireUnconsumed = false)
            // An excluded child sees the touch first.
            drawer.regionTouched = !drawer.regionExcluded
            drawer.regionExcluded = false
        }
    }

// Editors inside a swipe region keep their text drags, as the web's and Apple's excluded starts.
internal fun Modifier.browseSwipeExcluded(drawer: SlideDrawer?): Modifier =
    if (drawer == null) this else pointerInput(drawer) {
        awaitEachGesture {
            awaitFirstDown(requireUnconsumed = false)
            drawer.regionExcluded = true
        }
    }

/**
 * Tracks the finger on the unmoving container, since the layers it moves would otherwise move
 * their own touch coordinates. `direction` is the finger's way to open (right for Browse, left
 * for the member list); `travel` is the drawer's full travel for the container's width, Browse's
 * ([browseTravel]) by default, so the layer follows the finger 1:1.
 *
 * Until the finger passes the touch slop sideways this watches the Main pass, after the
 * timeline, channel list, sliders and text, so their own drags win; a consumed move belongs to
 * them. Once it slides, it takes every move first (Initial pass) and consumes it, which cancels
 * the press under the finger (an image, file, link, author, reaction or row never opens on
 * release, and no long-press menu opens) and keeps lists from scrolling along.
 */
internal fun Modifier.browseDrag(
    drawer: SlideDrawer, direction: Float = 1f, travel: (Density.(containerWidth: Float) -> Float)? = null, setOpen: (Boolean) -> Unit,
): Modifier =
    pointerInput(drawer) {
        val tracker = VelocityTracker()
        val density: Density = this
        awaitEachGesture {
            val down = awaitFirstDown(requireUnconsumed = false)
            val allowed = drawer.regionTouched
            drawer.regionTouched = false
            drawer.regionExcluded = false
            if (!allowed) return@awaitEachGesture
            val slop = viewConfiguration.touchSlop
            fun width() = (travel?.invoke(density, size.width.toFloat()) ?: browseTravel(size.width.toFloat())).coerceAtLeast(1f)
            tracker.resetTracking()
            tracker.addPointerInputChange(down)
            var opening = true
            var start = 0f
            var grabbedAt = 0f
            while (true) {
                val event = awaitPointerEvent()
                val change = event.changes.firstOrNull { it.id == down.id } ?: return@awaitEachGesture
                if (!change.pressed || event.changes.size != 1 || change.isConsumed) return@awaitEachGesture
                tracker.addPointerInputChange(change)
                val dx = (change.position.x - down.position.x) * direction
                val dy = change.position.y - down.position.y
                if (abs(dy) > slop && abs(dy) > abs(dx)) return@awaitEachGesture
                if (abs(dx) <= slop) continue
                if (abs(dx) < abs(dy) * 1.5f) return@awaitEachGesture
                opening = drawer.target < 0.5f
                // A resting drawer only moves toward its other side; a sliding one can be pushed either way.
                val sliding = drawer.value > 0f && drawer.value < 1f
                if (!sliding && (dx > 0) != opening) return@awaitEachGesture
                change.consume()
                // Grab it where it is now, even mid-slide, and follow the finger 1:1 past the slop.
                start = drawer.value
                grabbedAt = if (dx > 0) slop else -slop
                drawer.dragging = true
                drawer.drag(start + (dx - grabbedAt) / width())
                break
            }
            var dx = grabbedAt
            var velocity = 0f
            try {
                while (true) {
                    val event = awaitPointerEvent(PointerEventPass.Initial)
                    val change = event.changes.firstOrNull { it.id == down.id } ?: break
                    tracker.addPointerInputChange(change)
                    dx = (change.position.x - down.position.x) * direction
                    if (!change.pressed) {
                        velocity = tracker.calculateVelocity().x * direction
                        change.consume()
                        break
                    }
                    // A second finger settles the slide from where it is.
                    if (event.changes.size != 1) { event.changes.forEach { it.consume() }; break }
                    change.consume()
                    drawer.drag(start + (dx - grabbedAt) / width())
                }
            } finally {
                drawer.dragging = false
                val release = drawerRelease(
                    open = !opening, position = drawer.value, velocity = velocity, distance = dx, travel = width(),
                    flingSpeed = 300.dp.toPx(), flingDistance = 40.dp.toPx(),
                )
                drawer.settle(release.open, release.timing)
                setOpen(release.open)
            }
            // The rest of the gesture stays the drawer's, so no list or press beneath picks it up.
            while (currentEvent.changes.any { it.pressed }) {
                awaitPointerEvent(PointerEventPass.Initial).changes.forEach { it.consume() }
            }
        }
    }
