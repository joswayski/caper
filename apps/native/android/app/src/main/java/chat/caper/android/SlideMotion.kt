package chat.caper.android

import kotlin.math.abs
import kotlin.math.max
import kotlin.math.min
import kotlin.math.roundToInt

// Pure timing and release rules for the phone drawers and the media viewer's swipe-to-close,
// kept free of Android and Compose so JVM tests cover them. Values match web `releaseTiming`
// (`apps/web/src/spaces/browseTransition.ts`) and Apple's `DrawerRelease`.

/** Web `SETTLE`: a button, Back or channel choice slides a whole drawer in this long. */
internal const val DRAWER_SETTLE_MILLIS = 320
/** The shortest slide, so a nearly finished one still reads as motion. */
internal const val DRAWER_MIN_MILLIS = 160

/** How long a button's slide takes for the [remaining] share of the drawer's travel (web members drawer). */
internal fun drawerSettleMillis(remaining: Float): Int =
    max(DRAWER_MIN_MILLIS, (DRAWER_SETTLE_MILLIS * abs(remaining).coerceAtMost(1f)).roundToInt())

/**
 * A release curve that carries on from a finger: [durationMillis] long, starting at [slope] times
 * its average speed (`cubic-bezier(0.2, 0.2 × slope, 0.35, 1)`, which cannot overshoot).
 */
internal data class ReleaseTiming(val durationMillis: Int, val slope: Float)

/**
 * Web `releaseTiming`: [remaining] is the distance left and [speed] the finger's speed toward the
 * end, in the same unit per second. A held or reversing finger starts at rest; a fling keeps its
 * speed, capped at three times the average, without accelerating a held drag or slamming a short
 * remainder into the edge. Every curve lands at rest.
 */
internal fun releaseTiming(
    remaining: Float, speed: Float,
    minMillis: Int = DRAWER_MIN_MILLIS, maxMillis: Int = DRAWER_SETTLE_MILLIS,
): ReleaseTiming {
    val left = abs(remaining)
    val duration = if (speed > 0f && left > 0f) (3000f * left / speed).coerceIn(minMillis.toFloat(), maxMillis.toFloat()) else maxMillis.toFloat()
    val slope = if (left > 0f) (speed * duration / 1000f / left).coerceIn(0f, 3f) else 0f
    return ReleaseTiming(duration.roundToInt(), slope)
}

/** Where a released drawer swipe goes ([open]) and how ([timing]). */
internal data class DrawerRelease(val commit: Boolean, val open: Boolean, val timing: ReleaseTiming)

/**
 * Apple `DrawerRelease`: past halfway, or flung [flingDistance] toward the other view, a swipe
 * switches; a fling back keeps the view it started on.
 *
 * @param open the swipe started on the open drawer (or one sliding open).
 * @param position how far open it is now, from 0 to 1.
 * @param velocity the finger's speed toward open, in pixels per second.
 * @param distance how far the finger moved toward open, in pixels.
 * @param travel the drawer's full travel, in pixels.
 * @param flingSpeed the speed that counts as a fling, in pixels per second (300dp/s).
 * @param flingDistance how far a fling must have moved, in pixels (40dp).
 */
internal fun drawerRelease(
    open: Boolean, position: Float, velocity: Float, distance: Float, travel: Float,
    flingSpeed: Float, flingDistance: Float,
): DrawerRelease {
    val toward = if (open) -velocity else velocity
    val travelled = abs(position - if (open) 1f else 0f)
    val commit = toward > -flingSpeed && (travelled >= 0.5f || (toward >= flingSpeed && abs(distance) >= flingDistance))
    val endsOpen = commit != open
    val remaining = (if (endsOpen) 1f else 0f) - position
    // Toward the end it settles on, in travels per second.
    val speed = (if (remaining > 0f) velocity else -velocity) / max(travel, 1f)
    return DrawerRelease(commit, endsOpen, releaseTiming(remaining, speed))
}

/** The phone member list's travel: its width (at most [panelMax], inset by [inset]) plus [margin] for its shadow. */
internal fun membersTravel(containerWidth: Float, panelMax: Float, inset: Float, margin: Float): Float =
    max(1f, min(panelMax, containerWidth - inset) + margin)

/**
 * Whether a swipe down on the media viewer closes it: flung down, or dragged past [dismissAt]
 * without being flung back up.
 */
internal fun viewerDismisses(offset: Float, velocity: Float, dismissAt: Float, flingAt: Float): Boolean =
    velocity >= flingAt || (offset >= dismissAt && velocity > -flingAt)
