package chat.caper.android

import chat.caper.android.model.Account
import chat.caper.android.model.AppUiState
import chat.caper.android.model.Channel
import chat.caper.android.model.SessionScreen
import chat.caper.android.model.Space
import chat.caper.android.model.SpaceDetail
import chat.caper.android.data.CaperApi
import kotlinx.coroutines.test.runTest
import okhttp3.mockwebserver.MockResponse
import okhttp3.mockwebserver.MockWebServer
import org.junit.Assert.assertEquals
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
        assertTrue("text navigation does not cancel a voice target", intent.isCurrent(current.copy(selectedChannel = channel.copy(id = "channel0002")), 7))
        assertFalse(intent.isCurrent(current.copy(selectedSpace = SpaceDetail(space, emptyList(), emptyList())), 7))
        assertFalse(intent.isCurrent(current.copy(deniedVoiceChannels = setOf(channel.id)), 7))
        assertFalse(intent.isCurrent(current.copy(selectedSpace = SpaceDetail(space.copy(id = "space000002"), listOf(channel), emptyList())), 7))
        assertFalse("cached listing is insufficient after grant removal", intent.isCurrent(current, 7, emptySet()))
        assertFalse("fresh access does not override logout epoch", intent.isCurrent(current, 8, setOf(channel.id)))
        assertTrue("fresh target permission survives browsing another text channel",
            intent.isCurrent(current.copy(selectedChannel = channel.copy(id = "channel0002")), 7, setOf(channel.id)))
    }

    @Test fun `fresh space access filters revoked target before replacing another call`() = runTest {
        val server = MockWebServer()
        try {
            val currentSpace = Space("space0000001", "Fixture Studio", account.id)
            val currentTarget = Channel("channel00002", currentSpace.id, "design", true)
            val current = home().copy(selectedSpace = SpaceDetail(currentSpace, listOf(currentTarget), emptyList()),
                selectedChannel = Channel("channel00001", currentSpace.id, "general", false))
            val join = intent.copy(spaceId = currentSpace.id, channelId = currentTarget.id)
            server.enqueue(MockResponse().setBody("""{"space":{"id":"space0000001","name":"Fixture Studio","ownerId":"account0001"},"channels":[{"id":"channel00001","spaceId":"space0000001","name":"general","private":false}],"members":[]}"""))
            val fresh = CaperApi(baseUrl = server.url("/").toString()).space("account-secret", currentSpace.id)
            assertEquals("Bearer account-secret", server.takeRequest().headers["Authorization"])
            assertTrue("cached target still exists locally", join.isCurrent(current, 7))
            assertFalse("fresh grant removal must not replace the healthy general call",
                join.isCurrent(current, 7, fresh.channels.mapTo(mutableSetOf()) { it.id }))
            assertFalse(join.isCurrent(current, 8, setOf(currentTarget.id)))
        } finally { server.close() }
    }

    private fun home() = AppUiState(
        screen = SessionScreen.Home,
        account = account,
        selectedSpace = SpaceDetail(space, listOf(channel), emptyList()),
        selectedChannel = channel,
    )
}
