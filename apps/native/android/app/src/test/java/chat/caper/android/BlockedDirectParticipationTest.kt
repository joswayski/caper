package chat.caper.android

import chat.caper.android.model.AppUiState
import chat.caper.android.model.BlockedAccount
import chat.caper.android.model.Channel
import chat.caper.android.model.DirectConversation
import chat.caper.android.model.DirectPeer
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class BlockedDirectParticipationTest {
    private val peer = DirectPeer("account00002", "mira", "Mira")
    private val channel = Channel("direct000001", "", "Mira", private = true, direct = true)
    private fun state(direct: DirectConversation, blocks: List<BlockedAccount> = emptyList()) = AppUiState(
        selectedChannel = channel,
        directConversations = listOf(direct),
        selectedDirectId = direct.id,
        blocks = blocks,
    )

    @Test fun `a block stops reacting, pinning and editing as well as sending`() {
        val accepted = DirectConversation("direct000001", peer, "4", "4", "accepted")
        assertTrue(state(accepted).canParticipate)
        assertFalse(state(accepted.copy(blocked = true)).canParticipate)
        // The block list can learn of a block before the conversation list refreshes.
        assertFalse(state(accepted, listOf(BlockedAccount(peer.id, peer.username, peer.displayName))).canParticipate)
        assertFalse(state(accepted.copy(status = "incoming")).canParticipate)
    }
}
