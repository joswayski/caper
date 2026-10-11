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
}
