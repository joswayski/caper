package chat.caper.android

import chat.caper.android.model.AppUiState
import chat.caper.android.model.Member
import org.junit.Assert.assertEquals
import org.junit.Assert.assertSame
import org.junit.Test

class ChannelGrantsTest {
    private val maya = Member("maya0000000a", "maya", "Maya", false)
    private val bob = Member("bob00000000a", "bob", "Bob", false)

    @Test fun `grants show only for the channel they were loaded for`() {
        val loaded = AppUiState(channelGrantsChannelId = "channelA0001", channelGrants = listOf(maya), pendingChannelInvitations = listOf(bob))
        assertEquals(listOf(maya) to listOf(bob), loaded.channelGrantsFor("channelA0001"))
        assertEquals(emptyList<Member>() to emptyList<Member>(), loaded.channelGrantsFor("channelB0001"))
        assertEquals(emptyList<Member>() to emptyList<Member>(), AppUiState().channelGrantsFor("channelA0001"))
    }

    @Test fun `a late reply for another channel changes nothing`() {
        // Settings for B opened (and cleared the list) while A's request was still out.
        val openB = AppUiState(channelGrantsChannelId = "channelB0001")
        assertSame(openB, openB.withChannelGrants("channelA0001") { _, _ -> listOf(maya) to emptyList() })
        val loaded = openB.withChannelGrants("channelB0001") { _, _ -> listOf(maya, bob) to emptyList() }
        assertEquals(listOf(maya, bob), loaded.channelGrantsFor("channelB0001").first)
        val removed = loaded.withChannelGrants("channelB0001") { members, invitations -> members.filter { it.id != maya.id } to invitations }
        assertEquals(listOf(bob), removed.channelGrantsFor("channelB0001").first)
    }
}
