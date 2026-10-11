package chat.caper.android

import chat.caper.android.data.FEEDBACK_EMPTY_NOTE
import chat.caper.android.data.emptyConversationNote
import chat.caper.android.data.offersAuthorBlock
import chat.caper.android.model.Account
import chat.caper.android.model.AppUiState
import chat.caper.android.model.BlockedAccount
import chat.caper.android.model.Channel
import chat.caper.android.model.ChatAuthor
import chat.caper.android.model.DirectConversation
import chat.caper.android.model.DirectPeer
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class BlockedDirectParticipationTest {
    private val me = Account("account00001", "me", "Me")
    private val peer = DirectPeer("account00002", "mira", "Mira")
    private val channel = Channel("direct000001", "", "Mira", private = true, direct = true)
    private fun state(direct: DirectConversation, blocks: List<BlockedAccount> = emptyList()) = AppUiState(
        selectedChannel = channel,
        directConversations = listOf(direct),
        selectedDirectId = direct.id,
        blocks = blocks,
    )
    private val accepted = DirectConversation("direct000001", peer, "4", "4", "accepted")
    // A team member's view: the peer is the person who sent feedback.
    private val feedback = accepted.copy(feedback = true)

    @Test fun `a block stops reacting, pinning and editing as well as sending`() {
        assertTrue(state(accepted).canParticipate)
        assertFalse(state(accepted.copy(blocked = true)).canParticipate)
        // The block list can learn of a block before the conversation list refreshes.
        assertFalse(state(accepted, listOf(BlockedAccount(peer.id, peer.username, peer.displayName))).canParticipate)
        assertFalse(state(accepted.copy(status = "incoming")).canParticipate)
    }

    @Test fun `a feedback conversation is never locked by your block list`() {
        // A team member may have blocked the person personally; the server allows the conversation regardless.
        assertTrue(state(feedback, listOf(BlockedAccount(peer.id, peer.username, peer.displayName))).canParticipate)
        assertTrue(state(feedback.copy(blocked = true)).canParticipate)
        assertTrue(state(feedback).feedbackOpen)
        assertFalse(state(accepted).feedbackOpen)
        assertFalse(state(feedback).copy(selectedChannel = channel.copy(direct = false)).feedbackOpen)
    }

    @Test fun `Block and Unblock are offered in ordinary DMs only, never in feedback conversations`() {
        assertEquals(accepted, state(accepted).copy(account = me).blockableDirect)
        assertEquals(accepted.copy(blocked = true), state(accepted.copy(blocked = true)).copy(account = me).blockableDirect)
        assertNull(state(feedback).copy(account = me).blockableDirect)
        assertNull(state(feedback.copy(blocked = true)).copy(account = me).blockableDirect)
        // Requests answer in their own bar; your notes have nobody to block.
        assertNull(state(accepted.copy(status = "incoming")).copy(account = me).blockableDirect)
        assertNull(state(accepted.copy(peer = DirectPeer(me.id, "me", "Me"))).copy(account = me).blockableDirect)

        // Message actions: "Block @author" for another account's message, except in a feedback conversation.
        val author = ChatAuthor(peer.id, peer.displayName, false)
        assertTrue(offersAuthorBlock(state(accepted).copy(account = me), author))
        assertFalse(offersAuthorBlock(state(feedback).copy(account = me), author))
        assertFalse(offersAuthorBlock(state(feedback).copy(account = me), ChatAuthor("account00003", "Teammate", false)))
        assertFalse(offersAuthorBlock(state(accepted).copy(account = me), ChatAuthor(me.id, me.displayName!!, false)))
        assertFalse(offersAuthorBlock(state(accepted).copy(account = me), ChatAuthor("guest0000001", "Guest", true)))
        assertFalse(offersAuthorBlock(state(accepted), author))
    }

    @Test fun `empty conversations explain who can read them`() {
        assertEquals("Only you and Mira can read this conversation.", emptyConversationNote(state(accepted)))
        assertEquals(FEEDBACK_EMPTY_NOTE, emptyConversationNote(state(feedback)))
        assertEquals(
            "The Caper team reads and replies here. Mention @feedback anywhere to send a message or its thread here.",
            emptyConversationNote(state(feedback)),
        )
        assertEquals("Start the conversation in #general.",
            emptyConversationNote(AppUiState(selectedChannel = Channel("channel00001", "space0000001", "general", private = false))))
    }
}
