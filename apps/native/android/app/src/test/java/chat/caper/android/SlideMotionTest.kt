package chat.caper.android

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class SlideMotionTest {
    private val fling = 300f
    private val flingDistance = 40f
    private val travel = 400f

    private fun release(open: Boolean, position: Float, velocity: Float, distance: Float) =
        drawerRelease(open, position, velocity, distance, travel, fling, flingDistance)

    @Test fun buttonSlidesTakeWebSettleForTheWholeTravelAndShortenWithDistance() {
        assertEquals(320, drawerSettleMillis(1f))
        assertEquals(320, drawerSettleMillis(-1f))
        assertEquals(240, drawerSettleMillis(0.75f))
        // A nearly finished slide still reads as motion.
        assertEquals(160, drawerSettleMillis(0.1f))
        assertEquals(160, drawerSettleMillis(0f))
    }

    @Test fun aHeldReleaseGetsTheFullFinishFromRest() {
        val timing = releaseTiming(0.4f, 0f)
        assertEquals(320, timing.durationMillis)
        assertEquals(0f, timing.slope, 0f)
    }

    @Test fun aReversingFingerStartsAtRest() {
        assertEquals(0f, releaseTiming(0.4f, -2f).slope, 0f)
    }

    @Test fun aFlingCarriesItsSpeedIntoTheCurve() {
        // 0.5 travels left at 4 travels/s: 375ms clamps to 320ms; the curve starts at 4 × 0.32 / 0.5 = 2.56× its average.
        val timing = releaseTiming(0.5f, 4f)
        assertEquals(320, timing.durationMillis)
        assertEquals(2.56f, timing.slope, 0.001f)
        // Fast: 3 × 0.5 / 20 = 75ms clamps to 160ms, and the slope caps at 3 so a short remainder can't slam into the edge.
        val fast = releaseTiming(0.5f, 20f)
        assertEquals(160, fast.durationMillis)
        assertEquals(3f, fast.slope, 0f)
        // Unclamped, the curve starts exactly at the finger's speed: slope 3 over 3 × distance / speed.
        val matched = releaseTiming(0.5f, 6f)
        assertEquals(250, matched.durationMillis)
        assertEquals(3f, matched.slope, 0.001f)
    }

    @Test fun releaseCurvesNeverOvershoot() {
        // cubic-bezier(0.2, 0.2 × slope, 0.35, 1) stays within 0…1 while slope is at most 5.
        for (speed in listOf(-10f, 0f, 0.5f, 3f, 50f, 10_000f)) for (left in listOf(0.01f, 0.3f, 1f)) {
            val slope = releaseTiming(left, speed).slope
            assertTrue(slope in 0f..3f)
            assertTrue(0.2f * slope <= 1f)
        }
    }

    @Test fun pastHalfwayCommits() {
        val opened = release(open = false, position = 0.55f, velocity = 0f, distance = 220f)
        assertTrue(opened.commit)
        assertTrue(opened.open)
        val stayed = release(open = false, position = 0.45f, velocity = 0f, distance = 180f)
        assertFalse(stayed.commit)
        assertFalse(stayed.open)
    }

    @Test fun aFlingTowardTheOtherViewCommitsOnceItMovedFortyPixels() {
        assertTrue(release(open = false, position = 0.15f, velocity = 400f, distance = 60f).open)
        // Too short a flick doesn't switch views.
        assertFalse(release(open = false, position = 0.05f, velocity = 400f, distance = 20f).open)
        // Closing: the finger moves toward closed (negative toward open).
        assertFalse(release(open = true, position = 0.85f, velocity = -400f, distance = -60f).open)
    }

    @Test fun aFlingBackKeepsTheStartingViewEvenPastHalfway() {
        val back = release(open = false, position = 0.7f, velocity = -400f, distance = 280f)
        assertFalse(back.commit)
        assertFalse(back.open)
        // Settles closed at the finger's speed: 400px/s over 400px is 1 travel/s toward 0.
        assertEquals(320, back.timing.durationMillis)
        assertEquals(1f * 0.32f / 0.7f, back.timing.slope, 0.001f)
    }

    @Test fun aSlideGrabbedMidwayGoesToTheNearerSide() {
        // Sliding open (so "open") and caught at 0.3: it closes, rather than springing back open.
        assertFalse(release(open = true, position = 0.3f, velocity = 0f, distance = 0f).open)
        assertTrue(release(open = true, position = 0.7f, velocity = 0f, distance = 0f).open)
    }

    @Test fun membersTravelIsKnownBeforeThePanelIsMeasured() {
        // A 390-wide phone: the 280 panel plus its 32 margin, not a 1px travel that jumps open.
        assertEquals(312f, membersTravel(390f, 280f, 8f, 32f), 0f)
        // Narrower than the panel's widest: the panel fills the width beside its inset.
        assertEquals(272f, membersTravel(248f, 280f, 8f, 32f), 0f)
        assertEquals(1f, membersTravel(0f, 0f, 8f, 0f), 0f)
    }

    @Test fun viewerClosesPastTheThresholdOrOnAFlingDown() {
        assertTrue(viewerDismisses(offset = 130f, velocity = 0f, dismissAt = 120f, flingAt = 1000f))
        assertFalse(viewerDismisses(offset = 100f, velocity = 0f, dismissAt = 120f, flingAt = 1000f))
        assertTrue(viewerDismisses(offset = 10f, velocity = 1200f, dismissAt = 120f, flingAt = 1000f))
        // Dragged far, then flung back up: it stays.
        assertFalse(viewerDismisses(offset = 200f, velocity = -1500f, dismissAt = 120f, flingAt = 1000f))
    }
}
