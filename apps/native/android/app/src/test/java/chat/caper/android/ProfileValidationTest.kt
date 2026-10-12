package chat.caper.android

import chat.caper.android.model.*
import org.junit.Assert.*
import org.junit.Test

class ProfileValidationTest {
    @Test fun `saved identity replaces retained self snapshots and preserves the conversation`() {
        val own = Member("self", "old_user", "Old Name", true)
        val other = own.copy(id = "other", owner = false)
        val author = ChatAuthor(own.id, own.displayName, false)
        val original = ChatMessage("original", "channel", "7", author, ChatContent(1, "text", "Keep @old_user"), "now", "client")
        val message = original.copy(id = "wrapper", author = author.copy(id = other.id),
            pin = MessagePin(author, "now"), thread = ThreadSummary(1, listOf(author, author.copy(id = other.id)), "7"),
            forward = MessageForward(original, "7"))
        val detail = SpaceDetail(Space("space", "Space", own.id), emptyList(), listOf(own, other))
        val state = AppUiState(account = Account(own.id, "new_user", "New Name"), selectedSpace = detail,
            selectedChannel = Channel("channel", "space", "general", false), channelGrants = listOf(own, other),
            messages = listOf(message), pinnedMessages = listOf(message),
            pendingMessage = PendingMessageUi("pending", "rejected draft", author, "now", rejected = true))
        val displayed = state.withAccountProfile()
        assertEquals(listOf("New Name", "Old Name"), displayed.selectedSpace!!.members.map { it.displayName })
        assertEquals("new_user", displayed.channelGrants[0].username)
        assertEquals("Old Name", displayed.messages[0].author.name)
        assertEquals("New Name", displayed.messages[0].pin!!.author.name)
        assertEquals(listOf("New Name", "Old Name"), displayed.messages[0].thread!!.participants.map { it.name })
        assertEquals("New Name", displayed.messages[0].forward!!.message!!.author.name)
        assertEquals("Keep @old_user", displayed.messages[0].content.text)
        assertEquals(state.selectedChannel, displayed.selectedChannel)
        assertEquals("pending", displayed.pendingMessage!!.clientMessageId)
        assertTrue(displayed.pendingMessage!!.rejected)
        assertEquals("New Name", displayed.pendingMessage!!.author!!.name)
    }

    @Test fun `member usernames use profile username rules`() {
        assertTrue(usernameValid("abc"))
        assertTrue(usernameValid("person_123"))
        assertFalse(usernameValid("ab"))
        assertFalse(usernameValid("Upper"))
        assertFalse(usernameValid("person-name"))
        assertFalse(usernameValid("a".repeat(33)))
    }

    @Test fun `profile accepts Unicode scalars but rejects controls and invalid usernames`() {
        assertTrue(profileValid("ab_", "🪴".repeat(64)))
        assertFalse(profileValid("ab_", "🪴".repeat(65)))
        assertFalse(profileValid("ab", "Jose"))
        assertFalse(profileValid("a".repeat(33), "Jose"))
        assertFalse(profileValid("a-b", "Jose"))
        assertFalse(profileValid("ab_", " \t"))
        assertFalse(profileValid("ab_", "Jo\u0000se"))
        assertFalse(profileValid("ab_", "Jo\u0085se"))
    }
}
