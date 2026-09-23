package chat.caper.android

import chat.caper.android.model.Account
import chat.caper.android.model.AppUiState
import chat.caper.android.model.Channel
import chat.caper.android.model.SessionScreen
import chat.caper.android.model.Space
import chat.caper.android.model.SpaceDetail
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class VoiceJoinIntentTest {
    private val account = Account("account0001", "fixture", "Fixture Owner")
    private val space = Space("space000001", "Fixture Studio", "account0001")
    private val channel = Channel("channel0001", space.id, "general", false)
    private val intent = VoiceJoinIntent(
        channel.id, space.id, channel.name, space.name, account.displayName!!,
        account.id, accountEpoch = 7, demo = false,
    )

    @Test fun `permission result requires unchanged account epoch and context`() {
        val current = home()
        assertTrue(intent.isCurrent(current, 7))
        assertFalse("logout and login as same account must invalidate permission", intent.isCurrent(current, 8))
        assertFalse(intent.isCurrent(current.copy(selectedChannel = channel.copy(id = "channel0002")), 7))
        assertFalse(intent.isCurrent(current.copy(selectedSpace = SpaceDetail(space.copy(id = "space000002"), listOf(channel), emptyList())), 7))
    }

    private fun home() = AppUiState(
        screen = SessionScreen.Home,
        account = account,
        selectedSpace = SpaceDetail(space, listOf(channel), emptyList()),
        selectedChannel = channel,
    )
}
