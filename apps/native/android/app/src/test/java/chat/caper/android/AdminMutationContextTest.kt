package chat.caper.android

import chat.caper.android.model.Channel
import chat.caper.android.model.Space
import chat.caper.android.model.SpaceDetail
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.async
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Test

class AdminMutationContextTest {
    @Test fun `delayed member response after space switch cannot mutate or invoke callback`() = runTest {
        val response = CompletableDeferred<String>()
        val context = AdminMutationContext(4, "space000001")
        var selected = detail("space000001", "channel0001")
        var member = "unchanged"
        var callbacks = 0
        val request = async {
            val value = response.await()
            if (!context.isCurrent(4, selected)) return@async
            member = value
            callbacks++
        }

        selected = detail("space000002", "channel0002")
        response.complete("late-private-member")
        request.await()

        assertEquals("unchanged", member)
        assertEquals(0, callbacks)
    }

    @Test fun `logout and removed channel invalidate delayed mutations`() {
        val selected = detail("space000001", "channel0001")
        assertFalse(AdminMutationContext(4, "space000001").isCurrent(5, selected))
        assertFalse(AdminMutationContext(4, "space000001", "removed00001").isCurrent(4, selected))
    }

    private fun detail(space: String, channel: String) = SpaceDetail(
        Space(space, "Space", "owner000001"),
        listOf(Channel(channel, space, "general", false)),
        emptyList(),
    )
}
