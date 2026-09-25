package chat.caper.android

import chat.caper.android.model.*
import org.junit.Assert.*
import org.junit.Test

class VoiceAvailabilityTest {
    @Test fun `join labels follow web while availability is unknown or false`() {
        assertEquals("Checking voice availability…", voiceJoinUnavailableLabel(null))
        assertEquals("Joining is not available at this time.", voiceJoinUnavailableLabel(false))
        assertNull(voiceJoinUnavailableLabel(true))
    }

    @Test fun `viewed channel media root decides availability`() {
        val general = Channel("chan00000001", "space0000001", "general", false)
        val demo = SpaceDetail(Space("space0000001", "General", demo = true), listOf(general), emptyList())
        val planning = Channel("chan00000002", "space0000002", "planning", false)
        val studio = SpaceDetail(Space("space0000002", "Studio"), listOf(planning), emptyList())
        val known = mapOf("" to true, "chan00000002" to false)
        assertEquals(true, AppUiState(selectedSpace = demo, selectedChannel = general, voiceAvailability = known).voiceAvailable)
        assertEquals(false, AppUiState(selectedSpace = studio, selectedChannel = planning, voiceAvailability = known).voiceAvailable)
        assertNull(AppUiState(selectedSpace = studio, selectedChannel = planning).voiceAvailable)
    }
}
