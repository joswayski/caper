package chat.caper.android

import org.junit.Assert.*
import org.junit.Test

/** Mirrors web's `chat/arrivals.test.ts`. */
class ArrivalsTest {
    private var time = 0L
    private val arrivals = Arrivals { time }

    @Test fun `does not animate a channel's first render or a channel switch`() {
        arrivals.observe("channel", "a", listOf("1", "2"), true)
        assertFalse(arrivals.has("2"))
        arrivals.observe("channel", "b", listOf("3", "4"), true)
        assertFalse(arrivals.has("4"))
    }

    @Test fun `animates rows appended at the live end`() {
        arrivals.observe("channel", "a", listOf("1", "2"), true)
        arrivals.observe("channel", "a", listOf("1", "2", "3"), true)
        assertTrue(arrivals.has("3"))
        assertFalse(arrivals.has("2"))
    }

    @Test fun `does not animate an empty list's first messages`() {
        arrivals.observe("channel", "a", emptyList(), true)
        arrivals.observe("channel", "a", listOf("1", "2"), true)
        assertFalse(arrivals.has("1"))
    }

    @Test fun `ignores older pages and newer pages loaded towards the live end`() {
        arrivals.observe("channel", "a", listOf("5", "6"), false)
        arrivals.observe("channel", "a", listOf("3", "4", "5", "6"), false)
        arrivals.observe("channel", "a", listOf("3", "4", "5", "6", "7", "8"), true)
        assertFalse(listOf("3", "4", "7", "8").any(arrivals::has))
        arrivals.observe("channel", "a", listOf("1", "2", "3", "4", "5", "6", "7", "8"), true)
        assertFalse(arrivals.has("1"))
    }

    @Test fun `animates a confirmed send once, keeping its client message ID`() {
        arrivals.observe("channel", "a", listOf("1"), true)
        arrivals.observe("channel", "a", listOf("1", "sent"), true)
        assertTrue(arrivals.has("sent"))
        time += ARRIVAL_MS + 1
        arrivals.observe("channel", "a", listOf("1", "sent"), true)
        assertFalse(arrivals.has("sent"))
    }

    @Test fun `a confirmed row resumes the pending row's entrance`() {
        arrivals.observe("channel", "a", listOf("1"), true)
        arrivals.observe("channel", "a", listOf("1", "sent"), true)
        time += 120
        arrivals.observe("channel", "a", listOf("1", "sent"), true)
        assertEquals(120L, arrivals.elapsed("sent"))
        assertNull(arrivals.elapsed("1"))
    }

    @Test fun `keeps channel and thread lists apart`() {
        arrivals.observe("channel", "a", listOf("1"), true)
        arrivals.observe("thread", "root", listOf("r1"), true)
        arrivals.observe("thread", "root", listOf("r1", "r2"), true)
        arrivals.observe("channel", "a", listOf("1"), true)
        assertTrue(arrivals.has("r2"))
    }
}
