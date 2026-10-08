package chat.caper.android

import chat.caper.android.model.AppUiState
import chat.caper.android.model.Channel
import chat.caper.android.model.Member
import chat.caper.android.model.Space
import chat.caper.android.model.SpaceDetail
import org.junit.Assert.*
import org.junit.Test

class SpaceNavigationTest {
    private val general = Channel("channel00001", "space0000001", "general", private = false)
    private val design = Channel("channel00002", "space0000001", "design", private = false)
    private val preview = Channel("channel00003", "space0000001", "lobby", private = false, joined = false)
    private val channels = listOf(preview, general, design)
    private val detail = SpaceDetail(Space("space0000001", "Studio"), channels, emptyList())

    @Test fun `a space reopens its last channel while you are still in it`() {
        assertEquals(design, spaceLandingChannel(channels, preferred = null, remembered = design.id))
        // A requested channel wins, even one you only preview.
        assertEquals(preview, spaceLandingChannel(channels, preferred = preview.id, remembered = design.id))
        // A remembered channel you left, or one that is gone, falls back to the first joined channel.
        assertEquals(general, spaceLandingChannel(channels, preferred = null, remembered = preview.id))
        assertEquals(general, spaceLandingChannel(channels, preferred = null, remembered = "channel00009"))
        assertNull(spaceLandingChannel(listOf(preview), preferred = null, remembered = null))
    }

    @Test fun `choosing what is already shown changes nothing`() {
        val shown = AppUiState(selectedSpace = detail, selectedChannel = design)
        assertTrue(showsSpace(shown, detail.space.id))
        assertTrue(showsChannel(shown, design))
        assertFalse(showsChannel(shown, general))
        // Joining or leaving reopens the same channel with or without the composer.
        assertFalse(showsChannel(shown, design.copy(joined = false)))
        // A failed first load is retried, and a DM over the space is left for it.
        assertFalse(showsChannel(shown.copy(messagesError = "Messages are unavailable."), design))
        assertFalse(showsSpace(shown.copy(selectedDirectId = "direct000001"), detail.space.id))
        // A space still opening (no channel yet) can be chosen again.
        assertFalse(showsSpace(shown.copy(selectedChannel = null), detail.space.id))
    }

    @Test fun `invites are checked before they are sent`() {
        val members = listOf(Member("account00001", "mira", "Mira", owner = true))
        val invited = listOf(Member("account00002", "ade", "Ade", owner = false))
        assertEquals("Use 3–32 lowercase letters, numbers, or underscores.", memberInviteError("jo", members, invited, channel = false))
        assertEquals("This person is already in the space.", memberInviteError("mira", members, invited, channel = false))
        assertEquals("This person already has access to this channel.", memberInviteError("mira", members, invited, channel = true))
        assertEquals("This person already has a pending invitation.", memberInviteError("ade", members, invited, channel = false))
        assertNull(memberInviteError("sam_1", members, invited, channel = true))
    }

    @Test fun `server member errors use web copy`() {
        assertEquals("User not found. Check the username and try again.", memberAddError("user not found"))
        assertEquals("This person already has a pending invitation.", memberAddError("user already invited"))
        assertEquals("Network is unreachable", memberAddError("Network is unreachable"))
    }
}
