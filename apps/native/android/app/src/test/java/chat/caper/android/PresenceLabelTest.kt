package chat.caper.android

import org.junit.Assert.assertEquals
import org.junit.Test

class PresenceLabelTest {
    @Test fun `whole roster filters name or username and preserves private reader order`() {
        val members = (0..150).map { chat.caper.android.model.Member("$it", "user$it", "Member $it", false) }
        val detail = chat.caper.android.model.SpaceDetail(
            chat.caper.android.model.Space("space", "Space"), emptyList(), members,
            channelMembers = mapOf("private" to listOf("150")),
        )
        assertEquals(151, detail.memberOrder(null).size)
        assertEquals("150", detail.memberOrder("private").first().id)
        assertEquals(listOf("150"), detail.memberOrder("private", " USER150 ").map { it.id })
        assertEquals(listOf("150"), detail.memberOrder(null, " member 150 ").map { it.id })
        assertEquals(emptyList<String>(), detail.memberOrder(null, "missing").map { it.id })
        assertEquals(members, detail.memberOrder(null, "   "))
    }

    @Test fun `labels match the web PresenceDot`() {
        assertEquals("Online", presenceLabel("online", live = true))
        assertEquals("Idle", presenceLabel("idle", live = true))
        assertEquals("Offline (last known; reconnecting)", presenceLabel("offline", live = false))
        assertEquals("Status unavailable", presenceLabel(null, live = true))
    }
}
