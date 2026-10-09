package chat.caper.android

import org.junit.Assert.*
import org.junit.Test

class TimelineFollowTest {
    private val latest = 9 to PAST_END

    private fun TimelineFollow.next(frame: FollowFrame, ready: Boolean = true, near: Boolean = false, anchor: Pair<Int, Int>? = null) =
        followScroll(this, ready, frame, lastIndex = 9, near = { near }, anchor = { anchor })

    @Test fun `only a reader within the threshold of the last item follows`() {
        assertTrue(nearBottom(totalItems = 0, lastVisibleIndex = -1, lastVisibleEnd = 0, contentEnd = 800, threshold = 210))
        assertTrue(nearBottom(30, 29, lastVisibleEnd = 800, contentEnd = 800, threshold = 210))
        assertTrue(nearBottom(30, 29, lastVisibleEnd = 1010, contentEnd = 800, threshold = 210))
        assertFalse(nearBottom(30, 29, lastVisibleEnd = 1011, contentEnd = 800, threshold = 210))
        // The last item is not even laid out: the reader scrolled away.
        assertFalse(nearBottom(30, 24, lastVisibleEnd = 800, contentEnd = 800, threshold = 210))
        // A short conversation that fits the viewport.
        assertTrue(nearBottom(4, 3, lastVisibleEnd = 300, contentEnd = 800, threshold = 210))
    }

    @Test fun `an older page keeps the first visible row that moved in place`() {
        // The channel header stays at index 0; the first message moved from 1 to 21.
        val moved = mapOf("m1" to 21, "m2" to 22)
        val anchor = olderPageAnchor(listOf(Triple("header", 0, 0), Triple("m1", 1, 132), Triple("m2", 2, 300))) { key -> if (key == "header") 0 else moved[key] }
        assertEquals(21 to -132, anchor)
        // A row partly above the viewport keeps its hidden part hidden.
        assertEquals(21 to 40, olderPageAnchor(listOf(Triple("m1", 1, -40))) { moved[it] })
        assertNull(olderPageAnchor(listOf(Triple("root", 0, 0), Triple("r1", 1, 120))) { mapOf("root" to 0, "r1" to 1)[it] })
    }

    @Test fun `a conversation opens at the latest row once`() {
        val follow = TimelineFollow()
        assertNull(follow.next(FollowFrame(listOf("root")), ready = false))
        assertNull(follow.next(FollowFrame(emptyList())))
        assertFalse(follow.settled)
        assertEquals(latest, follow.next(FollowFrame(listOf("a", "b"))))
        assertTrue(follow.settled)
        assertNull(follow.next(FollowFrame(listOf("a", "b")), near = true))
    }

    @Test fun `a recreated list keeps its restored position`() {
        assertNull(TimelineFollow(settled = true).next(FollowFrame(listOf("a", "b", "c")), near = true))
    }

    @Test fun `new rows follow only near the bottom, your own send always`() {
        val follow = TimelineFollow().apply { next(FollowFrame(listOf("a"))) }
        assertNull(follow.next(FollowFrame(listOf("a", "b"))))
        assertEquals(latest, follow.next(FollowFrame(listOf("a", "b", "c")), near = true))
        assertEquals(latest, follow.next(FollowFrame(listOf("a", "b", "c"), pendingId = "p1")))
        assertNull(follow.next(FollowFrame(listOf("a", "b", "c"), pendingId = "p1")))
    }

    @Test fun `older pages anchor before following`() {
        val follow = TimelineFollow().apply { next(FollowFrame(listOf("c"))); next(FollowFrame(listOf("c"), loadingOlder = true)) }
        assertEquals(4 to -120, follow.next(FollowFrame(listOf("a", "b", "c")), near = true, anchor = 4 to -120))
        // A reloaded page that moved nothing visible still follows its new last row.
        follow.next(FollowFrame(listOf("a", "b", "c"), loadingOlder = true))
        assertEquals(latest, follow.next(FollowFrame(listOf("a", "b", "c", "d")), near = true))
    }

    @Test fun `go to message owns the viewport and historical context never follows`() {
        val follow = TimelineFollow().apply { next(FollowFrame(listOf("x", "y"))) }
        assertNull(follow.next(FollowFrame(listOf("a", "t", "b"), historical = true, jump = 1), near = true))
        assertNull(follow.next(FollowFrame(listOf("a", "t", "b"), historical = true, jump = 1), near = true))
        // Paging forward inside the window, and into the newest page, keeps the reader's place.
        assertNull(follow.next(FollowFrame(listOf("a", "t", "b"), historical = true, loadingNewer = true, jump = 1)))
        assertNull(follow.next(FollowFrame(listOf("a", "t", "b", "c"), historical = true, jump = 1), near = true))
        assertNull(follow.next(FollowFrame(listOf("a", "t", "b", "c"), historical = true, loadingNewer = true, jump = 1)))
        assertNull(follow.next(FollowFrame(listOf("a", "t", "b", "c", "d")), near = true))
        // A fresh jump in a list that never settled positions it instead of the latest row.
        val fresh = TimelineFollow()
        assertNull(fresh.next(FollowFrame(listOf("a", "t"), jump = 2)))
        assertTrue(fresh.settled)
    }

    @Test fun `returning to the newest page reveals the latest row`() {
        val back = TimelineFollow().apply { next(FollowFrame(listOf("a"))); next(FollowFrame(listOf("a", "t"), historical = true, jump = 1)) }
        assertEquals(latest, back.next(FollowFrame(listOf("x", "y", "z"))))
        // Sending from history reveals the send, then the newest page, even through a forward load.
        val sent = TimelineFollow().apply { next(FollowFrame(listOf("a"))); next(FollowFrame(listOf("a", "t"), historical = true, jump = 1)) }
        assertEquals(latest, sent.next(FollowFrame(listOf("a", "t"), pendingId = "p", historical = true, jump = 1)))
        assertNull(sent.next(FollowFrame(listOf("a", "t", "p"), historical = true, loadingNewer = true, jump = 1)))
        assertEquals(latest, sent.next(FollowFrame(listOf("x", "p"))))
    }
}
