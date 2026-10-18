package chat.caper.android

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class ThreadBackSwipeTest {
    private fun swipe() = ThreadBackSwipe(10f, 72f)

    @Test fun rightSwipeClosesOnlyAfterEnoughDistance() {
        val swipe = swipe()
        assertFalse(swipe.move(8f, 0f, false))
        assertTrue(swipe.move(25f, 2f, false))
        assertFalse(swipe.released(71f))
        assertTrue(swipe.released(72f))
        assertFalse(swipe.released(-80f))
    }

    @Test fun verticalLeftAndAmbiguousStartsCannotTurnIntoBack() {
        for ((dx, dy) in listOf(2f to 15f, -20f to 0f, 15f to 12f)) {
            val swipe = swipe()
            assertFalse(swipe.move(dx, dy, false))
            assertFalse(swipe.move(100f, 0f, false))
            assertFalse(swipe.released(100f))
        }
    }

    @Test fun scrollConsumptionOrMultiplePointersCancelWithoutReviving() {
        val unclaimed = swipe()
        assertFalse(unclaimed.move(30f, 0f, true))
        assertFalse(unclaimed.move(100f, 0f, false))
        assertFalse(unclaimed.released(100f))
        val claimed = swipe()
        assertTrue(claimed.move(30f, 0f, false))
        // Continue suppressing presses, but never close after interruption.
        assertTrue(claimed.move(90f, 0f, true))
        assertTrue(claimed.move(100f, 0f, false))
        assertFalse(claimed.released(100f))
    }
}
