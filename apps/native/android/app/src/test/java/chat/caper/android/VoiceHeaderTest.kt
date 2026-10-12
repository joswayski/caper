package chat.caper.android

import org.junit.Assert.assertEquals
import org.junit.Test

class VoiceHeaderTest {
    @Test fun `header summary names up to three people, then how many more`() {
        assertEquals("Maya is in voice", voicePeopleSummary(listOf("Maya")))
        assertEquals("Maya and Alex are in voice", voicePeopleSummary(listOf("Maya", "Alex")))
        assertEquals("Maya, Alex and Sam are in voice", voicePeopleSummary(listOf("Maya", "Alex", "Sam")))
        assertEquals("Maya, Alex, Sam and 2 more are in voice", voicePeopleSummary(listOf("Maya", "Alex", "Sam", "Jo", "Lee")))
    }

    @Test fun `phone subtitle counts members and people in voice, omitting what does not apply`() {
        assertEquals("12 members · 3 in voice", headerSubtitle(12, 3))
        assertEquals("1 member", headerSubtitle(1, 0))
        assertEquals("2 in voice", headerSubtitle(null, 2))
        assertEquals("", headerSubtitle(null, 0))
    }

    @Test fun `join glows only while others are in voice here and you can join or switch`() {
        assertEquals(true, headerJoinGlows(occupied = true, action = HeaderVoiceAction.Join, enabled = true))
        assertEquals(true, headerJoinGlows(occupied = true, action = HeaderVoiceAction.Switch, enabled = true))
        assertEquals(false, headerJoinGlows(occupied = false, action = HeaderVoiceAction.Join, enabled = true))
        assertEquals(false, headerJoinGlows(occupied = true, action = HeaderVoiceAction.Join, enabled = false))
        assertEquals(false, headerJoinGlows(occupied = true, action = HeaderVoiceAction.Joining, enabled = true))
        assertEquals(false, headerJoinGlows(occupied = true, action = HeaderVoiceAction.Leave, enabled = true))
    }
}
