package chat.caper.android.data

import org.junit.Assert.*
import org.junit.Test

class WhatsNewTest {
    private fun revision(sequence: Int) = sequence.toString().padStart(40, '0')
    private fun snapshot(last: Int, silent: Set<Int> = emptySet()): WhatsNewSnapshot = WhatsNewSnapshot(
        1, "android", revision(last), last.toLong(), (1..last).map {
            PlatformRelease(it.toLong(), revision(it), "2026-10-11T19:00:00Z",
                if (it in silent) emptyList() else listOf(ReleaseHighlight("Update $it", "Useful change $it.")))
        },
    )

    @Test fun `first install seeds quietly and a skipped update catches up newest first until dismissal`() {
        var ack: Long? = null
        val history = WhatsNewHistory({ ack }, { ack = it })
        history.receive(snapshot(1), "android", revision(1))
        assertEquals(1L, ack)
        assertTrue(history.unread.isEmpty())
        history.receive(snapshot(4, setOf(3)), "android", revision(4))
        assertEquals(listOf(4L, 2L), history.unread.map { it.sequence })
        assertEquals(1L, ack)
        val shown = history.snapshot!!.sequence
        history.acknowledge(shown)
        assertEquals(4L, ack)
        assertTrue(history.unread.isEmpty())
        assertEquals(listOf(4L, 2L, 1L), history.retained.map { it.sequence })
        val relaunched = WhatsNewHistory({ ack }, { ack = it })
        relaunched.receive(snapshot(4, setOf(3)), "android", revision(4))
        assertTrue(relaunched.unread.isEmpty())
    }

    @Test fun `silent updates advance and rollback or old sheet dismissal never lowers acknowledgement`() {
        var ack: Long? = 1L
        val history = WhatsNewHistory({ ack }, { ack = it })
        history.receive(snapshot(3, setOf(2, 3)), "android", revision(3))
        assertEquals(3L, ack)
        assertTrue(history.unread.isEmpty())
        history.receive(snapshot(1), "android", revision(1))
        history.acknowledge(1)
        assertEquals(3L, ack)
        history.receive(snapshot(4), "android", revision(4))
        assertEquals(listOf(4L), history.unread.map { it.sequence })
    }

    @Test fun `invalid snapshots never seed or replace the previous valid snapshot`() {
        var ack: Long? = null
        val history = WhatsNewHistory({ ack }, { ack = it })
        val value = snapshot(3)
        val invalid = listOf(
            value.copy(schema = 2), value.copy(platform = "ios"), value.copy(revision = revision(2)),
            value.copy(sequence = 4), value.copy(sequence = 9007199254740992L),
            value.copy(releases = value.releases.reversed()),
            value.copy(releases = value.releases + value.releases.last()),
        )
        invalid.forEach { assertThrows(IllegalArgumentException::class.java) { history.receive(it, "android", revision(3)) } }
        assertNull(ack)
        assertNull(history.snapshot)
        history.receive(value, "android", revision(3))
        assertThrows(IllegalArgumentException::class.java) { history.receive(snapshot(4), "android", revision(3)) }
        assertEquals(value, history.snapshot)
        assertEquals(3L, ack)
    }

    @Test fun `highlight bounds count Unicode code points rather than UTF16 units`() {
        val value = snapshot(1)
        val release = value.releases.single()
        val highlight = ReleaseHighlight("🙂".repeat(100), "e\u0301".repeat(250))
        value.copy(releases = listOf(release.copy(highlights = listOf(highlight)))).validated("android", revision(1))
        listOf(highlight.copy(title = highlight.title + "x"), highlight.copy(body = highlight.body + "x"), highlight.copy(body = "\u0000")).forEach {
            assertThrows(IllegalArgumentException::class.java) {
                value.copy(releases = listOf(release.copy(highlights = listOf(it)))).validated("android", revision(1))
            }
        }
        assertThrows(IllegalArgumentException::class.java) {
            value.copy(releases = listOf(release.copy(highlights = List(6) { highlight }))).validated("android", revision(1))
        }
    }
}
