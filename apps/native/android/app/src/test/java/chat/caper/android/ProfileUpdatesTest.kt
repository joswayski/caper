package chat.caper.android

import chat.caper.android.model.*
import org.junit.Assert.*
import org.junit.Test

class ProfileUpdatesTest {
    private val maya = "maya00000001"
    private val self = "self00000001"
    private val oldAsset = "OldPicture0000000001"
    private val newAsset = "NewPicture0000000001"
    private val newUrl = "https://cdn.caper.chat/assets/$newAsset?exp=1790000000&sig=n"
    private val update = ProfilePerson(maya, "maya_new", "Maya New", 312, newAsset, newUrl)

    private fun author(id: String = maya, name: String = "Maya") =
        ChatAuthor(id, name, false, 312, avatarAsset = oldAsset, avatarUrl = "https://cdn.caper.chat/assets/$oldAsset?sig=o")
    private fun member(id: String = maya, username: String = "maya", name: String = "Maya") =
        Member(id, username, name, false, 312, oldAsset, "https://cdn.caper.chat/assets/$oldAsset?sig=o")

    private fun state(): AppUiState {
        val other = author("ann000000001", "Ann")
        val original = ChatMessage("message00000", "channel00001", "5", author(), ChatContent(1, "text", "original"), "now", "client0")
        val message = ChatMessage("message00001", "channel00001", "7", other, ChatContent(1, "text", "Keep @maya"), "now", "client1",
            pin = MessagePin(author(), "now"), thread = ThreadSummary(2, listOf(author(), other), "7"),
            forward = MessageForward(original, "5"), replyTo = MessageReply("message00000", author(), "earlier", false))
        val dm = DirectConversation("direct000001", DirectPeer(maya, "maya", "Maya", 312, avatarAsset = oldAsset, avatarUrl = "x"), "0", "0")
        return AppUiState(
            account = Account(self, "me", "Me", avatarId = 1),
            selectedSpace = SpaceDetail(Space("space0000001", "Studio", self), emptyList(), listOf(member(self, "me", "Me"), member())),
            channelGrants = listOf(member()), pendingSpaceInvitations = listOf(member()), pendingChannelInvitations = listOf(member()),
            directConversations = listOf(dm), selectedDirectId = dm.id,
            selectedChannel = Channel(dm.id, "", "Maya", private = true, direct = true),
            people = listOf(Person(maya, "maya", "Maya", 312, oldAsset, "x")),
            blocks = listOf(BlockedAccount(maya, "maya", "Maya", 312)),
            messages = listOf(message), pinnedMessages = listOf(message),
            replyTargets = mapOf("channel" to MessageReply("message00001", author(), "hi", false)),
            typingAuthors = listOf(author()),
            feedback = FeedbackInboxUi(reports = listOf(FeedbackReport("report000001", "message00001", "message00001", "now",
                reporter = FeedbackReporter(maya, "maya", "Maya"), channel = FeedbackChannel("channel00001", "general")))),
            visit = VisitUi("channel00001", "space0000001", "Studio", "general", false, reporter = FeedbackReporter(maya, "maya", "Maya")),
        )
    }

    @Test fun `another person's update reaches every retained copy and nothing else`() {
        val before = state()
        val after = before.withPerson(update)

        fun Member.picture() = Triple(username, displayName, avatarAsset)
        assertEquals(Triple("maya_new", "Maya New", newAsset), after.selectedSpace!!.members[1].picture())
        assertEquals(before.selectedSpace!!.members[0], after.selectedSpace!!.members[0])
        listOf(after.channelGrants, after.pendingSpaceInvitations, after.pendingChannelInvitations).forEach {
            assertEquals(Triple("maya_new", "Maya New", newAsset), it.single().picture())
            assertEquals(newUrl, it.single().avatarUrl)
        }
        val peer = after.directConversations.single().peer
        assertEquals(listOf("maya_new", "Maya New", newAsset, newUrl), listOf(peer.username, peer.displayName, peer.avatarAsset, peer.avatarUrl))
        assertEquals("the open DM is titled with the new name", "Maya New", after.selectedChannel!!.name)
        assertEquals(before.selectedChannel!!.id, after.selectedChannel!!.id)
        assertEquals(newAsset, after.people!!.single().avatarAsset)
        assertEquals("maya_new", after.blocks.single().username)
        assertEquals(newAsset, after.blocks.single().avatarAsset)

        val message = after.messages.single()
        assertEquals("Ann", message.author.name)
        assertEquals(oldAsset, message.author.avatarAsset)
        assertEquals("Maya New", message.pin!!.author.name)
        assertEquals(listOf("Maya New", "Ann"), message.thread!!.participants.map { it.name })
        assertEquals(newAsset, message.thread!!.participants[0].avatarAsset)
        assertEquals(newUrl, message.replyTo!!.author.avatarUrl)
        assertEquals("Maya New", message.forward!!.message!!.author.name)
        assertEquals(newAsset, after.pinnedMessages.single().pin!!.author.avatarAsset)
        assertEquals("Maya New", after.replyTargets.getValue("channel").author.name)
        assertEquals("Maya New", after.typingAuthors.single().name)
        assertEquals("maya_new", after.feedback.reports.single().reporter.username)
        assertEquals(newAsset, after.visit!!.reporter!!.avatarAsset)

        // Text, sequences, cursors and your own account never change.
        assertEquals("Keep @maya", message.content.text)
        assertEquals(before.messages.single().seq, message.seq)
        assertEquals(before.account, after.account)
        assertEquals(before.selectedDirectId, after.selectedDirectId)
    }

    @Test fun `a removed picture is cleared everywhere, keeping the saved mascot`() {
        val removed = state().withPerson(ProfilePerson(maya, "maya", "Maya", 312))
        assertNull(removed.selectedSpace!!.members[1].avatarAsset)
        assertNull(removed.messages.single().pin!!.author.avatarUrl)
        assertNull(removed.directConversations.single().peer.avatarAsset)
        assertEquals(312, removed.messages.single().replyTo!!.author.avatarId)
    }

    @Test fun `your own update from another device changes the account and every copy of you`() {
        val mine = state().copy(messages = listOf(ChatMessage("message00002", "channel00001", "8", ChatAuthor(self, "Me", false, 1),
            ChatContent(1, "text", "mine"), "now", "client2")))
        val changed = mine.withPerson(ProfilePerson(self, "me_too", "Me Too", 1, newAsset, newUrl))
        assertEquals(listOf("me_too", "Me Too", newAsset, newUrl),
            changed.account!!.let { listOf(it.username, it.displayName, it.avatarAsset, it.avatarUrl) })
        assertEquals("Me Too", changed.messages.single().author.name)
        assertEquals("Me Too", changed.selectedSpace!!.members[0].displayName)
        // The derived own-profile refresh agrees with it.
        assertEquals(changed.selectedSpace, changed.withAccountProfile().selectedSpace)
        assertEquals(changed.messages, changed.withAccountProfile().messages)
    }

    @Test fun `cached pages, reactor lists and open cards follow a person`() {
        val history = ChatHistory(state().messages, "7", false, pinnedMessages = state().messages)
        assertEquals("Maya New", history.withPerson(update).pinnedMessages.single().pin!!.author.name)
        val reactors = ReactorList("message00001", "3", listOf(ReactorGroup("👍", listOf(Reactor(maya, "maya", "Maya"), Reactor("ann000000001", "ann", "Ann")))))
        val updated = reactors.withPerson(update)
        assertEquals(listOf("Maya New", "Ann"), updated.reactions.single().authors.map { it.displayName })
        assertEquals(newUrl, updated.reactions.single().authors[0].avatarUrl)
        assertEquals("3", updated.reactionSeq)
        assertSame("lists without them are reused", reactors, reactors.withPerson(update.copy(id = "zed000000001")))

        val before = state()
        val card = mentionCard(maya, "maya", before)
        assertEquals("Maya", card.displayName)
        val open = refreshedCard(card, before.withPerson(update))
        assertEquals(listOf("maya_new", "Maya New", newAsset), listOf(open.username, open.displayName, open.avatarAsset))
        val stranger = MentionCard("zed000000001", "zed", "Zed")
        assertEquals("unknown people keep the card they opened with", stranger, refreshedCard(stranger, before.withPerson(update)))
    }

    @Test fun `a refreshed list finds the copies a missed update left behind`() {
        val before = state()
        assertEquals("loaded messages and other lists still show the old profile", listOf(update), before.staleProfiles(listOf(update)))
        assertEquals("once applied nothing is left to repair", emptyList<ProfilePerson>(),
            before.withPerson(update, keepSignedUrls = true).staleProfiles(listOf(update)))

        val current = ProfilePerson(maya, "maya", "Maya", 312, oldAsset, "https://cdn.caper.chat/assets/$oldAsset?sig=fresh")
        assertEquals("a newly signed URL for the same picture is not a change", emptyList<ProfilePerson>(),
            before.copy(blocks = emptyList()).staleProfiles(listOf(current)))
        assertEquals("your own copies follow the account", emptyList<ProfilePerson>(),
            before.staleProfiles(listOf(ProfilePerson(self, "me_too", "Me Too", 1))))

        // A conversation cached before the update counts too, nested authors included.
        val cached = ChatHistory(before.messages, "7", false)
        val elsewhere = AppUiState(account = before.account)
        assertEquals(listOf(update), elsewhere.staleProfiles(listOf(update), listOf(cached)))
        assertEquals(emptyList<ProfilePerson>(), elsewhere.staleProfiles(listOf(update), listOf(cached.withPerson(update, keepSignedUrls = true))))
        assertEquals("people nobody here shows are ignored", emptyList<ProfilePerson>(),
            elsewhere.staleProfiles(listOf(update.copy(id = "zed000000001")), listOf(cached)))
    }

    @Test fun `a space update renames and re-icons that space everywhere, and ignores others`() {
        val icon = "https://cdn.caper.chat/assets/$newAsset?sig=i"
        val studio = Space("space0000001", "Studio", self, iconAsset = oldAsset, iconUrl = "x")
        val invited = Space("space0000002", "Invited", "ann000000001")
        val state = AppUiState(
            spaces = listOf(studio, Space("space0000003", "Other", self)), invitations = listOf(invited),
            selectedSpace = SpaceDetail(studio, emptyList(), emptyList()),
            visit = VisitUi("channel00001", "space0000001", "Studio", "general", false),
            feedback = FeedbackInboxUi(reports = listOf(
                FeedbackReport("report000001", "message00001", "message00001", "now", reporter = FeedbackReporter("ann000000001"),
                    channel = FeedbackChannel("channel00001", "general"), space = FeedbackSpace("space0000001", "Studio")),
                FeedbackReport("report000002", "message00002", "message00002", "now", reporter = FeedbackReporter("ann000000001"),
                    channel = FeedbackChannel("channel00003", "general"), space = FeedbackSpace("space0000003", "Other")),
            )),
        )
        val renamed = state.withSpace(Space("space0000001", "Friends", self, iconAsset = newAsset, iconUrl = icon))
        assertEquals("Friends", renamed.spaces[0].name)
        assertEquals(icon, renamed.spaces[0].iconUrl)
        assertEquals(state.spaces[1], renamed.spaces[1])
        assertEquals("Friends", renamed.selectedSpace!!.space.name)
        assertEquals(newAsset, renamed.selectedSpace!!.space.iconAsset)
        assertEquals("Friends", renamed.visit!!.spaceName)
        assertEquals("Feedback reports take the new name", listOf("Friends", "Other"), renamed.feedback.reports.map { it.space?.name })

        val cleared = renamed.withSpace(Space("space0000001", "Friends", self))
        assertNull(cleared.spaces[0].iconAsset)
        assertNull(cleared.selectedSpace!!.space.iconUrl)
        assertEquals("invitations take icons too", newAsset,
            state.withSpace(Space("space0000002", "Invited", "ann000000001", iconAsset = newAsset, iconUrl = icon)).invitations[0].iconAsset)
        assertEquals("a space you don't have changes nothing", state, state.withSpace(Space("space0000009", "Elsewhere", "x")))
    }
}
