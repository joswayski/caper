package chat.caper.android

import chat.caper.android.model.*
import kotlinx.serialization.json.Json
import org.junit.Assert.*
import org.junit.Test

class ProfilePictureTest {
    private val json = Json { ignoreUnknownKeys = true }
    private val asset = "AbCdEfGh12345678"
    private val url = "https://cdn.caper.chat/assets/$asset?exp=1790000000&sig=s"

    @Test fun `pictures are optional everywhere, so old payloads keep the mascot and initial`() {
        val old = json.decodeFromString<Account>("""{"id":"account00001","username":"jose","displayName":"Jose","avatarId":16}""")
        assertNull(old.avatarAsset)
        assertNull(pictureOf(old.avatarAsset, old.avatarUrl))
        val account = json.decodeFromString<Account>("""{"id":"account00001","avatarId":16,"avatarAsset":"$asset","avatarUrl":"$url"}""")
        assertEquals(Picture(asset, url), pictureOf(account.avatarAsset, account.avatarUrl))

        val spaces = json.decodeFromString<SpaceList>("""{"spaces":[{"id":"space0000001","name":"Studio","ownerId":"account00001","iconAsset":"$asset","iconUrl":"$url"},
            {"id":"space0000002","name":"Plain","ownerId":"account00001"}],
            "invitations":[{"id":"space0000003","name":"Invited","inviter":{"username":"ann","displayName":"Ann"},"iconAsset":"$asset","iconUrl":"$url"}],
            "limits":{"ownedSpaces":20,"totalSpaces":100,"channelsPerSpace":50}}""")
        assertEquals(asset, spaces.spaces[0].iconAsset)
        assertNull(spaces.spaces[1].iconUrl)
        assertEquals(url, spaces.invitations[0].iconUrl)

        val preview = json.decodeFromString<InvitePreview>("""{"code":"AbCdEfGh12","space":{"name":"Studio","iconAsset":"$asset","iconUrl":"$url"},
            "inviter":{"username":"ann","displayName":"Ann","avatarId":3,"avatarAsset":"$asset","avatarUrl":"$url"},"memberCount":2}""")
        assertEquals(url, preview.space.iconUrl)
        assertEquals(asset, preview.inviter.avatarAsset)
        assertNull(json.decodeFromString<InvitePreview>("""{"code":"AbCdEfGh12","space":{"name":"Studio"},"inviter":{"username":"ann","displayName":"Ann"}}""").space.iconAsset)

        val message = json.decodeFromString<ChatMessage>("""{"id":"message00001","channelId":"channel00001","seq":"9",
            "author":{"id":"ann000000001","name":"Ann","isGuest":false,"avatarId":3,"avatarAsset":"$asset","avatarUrl":"$url"},
            "content":{"version":1,"type":"text","text":"hi"},"createdAt":"2026-10-03T00:00:00Z","clientMessageId":"00000000-0000-4000-8000-000000000009",
            "replyTo":{"id":"message00000","author":{"id":"bob000000001","name":"Bob","isGuest":false},"text":"yo","hasAttachments":false}}""")
        assertEquals(url, message.author.avatarUrl)
        assertNull(message.replyTo!!.author.avatarAsset)

        val members = json.decodeFromString<MemberList>("""{"members":[{"id":"ann000000001","username":"ann","displayName":"Ann","owner":true,"avatarAsset":"$asset","avatarUrl":"$url"}],
            "invitations":[{"id":"bob000000001","username":"bob","displayName":"Bob","owner":false}]}""")
        assertEquals(asset, members.members[0].avatarAsset)
        assertNull(members.invitations[0].avatarAsset)

        val voice = json.decodeFromString<SpectatorSnapshot>("""{"participants":[{"id":"p1","name":"Ann","muted":false,"deafened":false,"avatarId":3,"avatarAsset":"$asset","avatarUrl":"$url"}],"revision":1}""")
        assertEquals(url, voice.participants[0].asParticipant().avatarUrl)
        assertEquals(asset, json.decodeFromString<MediaSnapshot>("""{"participants":[{"id":"p1","name":"Ann","muted":false,"deafened":false,"tracks":[],"avatarAsset":"$asset","avatarUrl":"$url"}]}""")
            .participants[0].avatarAsset)
        assertEquals(asset, json.decodeFromString<ReactorList>("""{"messageId":"message00001","reactionSeq":"3","reactions":[{"emoji":"👍","authors":[{"id":"ann000000001","avatarAsset":"$asset","avatarUrl":"$url"}]}]}""")
            .reactions[0].authors[0].avatarAsset)
        assertEquals(asset, json.decodeFromString<PeopleList>("""{"people":[{"id":"ann000000001","username":"ann","displayName":"Ann","avatarAsset":"$asset","avatarUrl":"$url"}]}""").people[0].avatarAsset)
        assertEquals(asset, json.decodeFromString<DirectConversationList>("""{"conversations":[{"id":"direct000001","peer":{"id":"ann000000001","username":"ann","displayName":"Ann","avatarAsset":"$asset","avatarUrl":"$url"},"lastSeq":"0","readSeq":"0"}]}""")
            .conversations[0].peer.avatarAsset)
        assertEquals(asset, json.decodeFromString<FeedbackReporter>("""{"id":"ann000000001","avatarAsset":"$asset","avatarUrl":"$url"}""").avatarAsset)
        assertEquals(asset, json.decodeFromString<PictureCompletion>("""{"id":"$asset","kind":"image","account":{"id":"account00001","avatarAsset":"$asset","avatarUrl":"$url"}}""").account!!.avatarAsset)
    }

    @Test fun `profile cards and mention suggestions carry other people's pictures`() {
        val ann = Member("ann000000001", "ann", "Ann", false, 3, asset, url)
        val channel = Channel("channel00001", "space0000001", "general", false)
        val state = AppUiState(selectedSpace = SpaceDetail(Space("space0000001", "Studio"), listOf(channel), listOf(ann)), selectedChannel = channel)
        val card = mentionCard(ann.id, "ann", state)
        assertEquals(Picture(asset, url), pictureOf(card.avatarAsset, card.avatarUrl))
        assertEquals(url, authorCard(ChatAuthor("zed000000001", "Zed", false, 4, avatarAsset = asset, avatarUrl = url), state).avatarUrl)
        assertEquals(asset, mentionSource(state).people.single().avatarAsset)
    }

    @Test fun `only a usable asset and web url make a picture`() {
        assertEquals(Picture(asset, url), pictureOf(asset, url))
        assertNull(pictureOf(asset, null))
        assertNull(pictureOf(null, url))
        assertNull(pictureOf("", url))
        assertNull(pictureOf(asset, "javascript:alert(1)"))
        assertNull(pictureOf(asset, "file:///data/secret.png"))
        assertNull(pictureOf("a b", url))
        assertNotNull(pictureOf(asset, "http://127.0.0.1:3001/assets/$asset"))
    }

    @Test fun `a new picture reaches every retained snapshot of you, and removal brings back the mascot`() {
        val own = Member("self00000001", "jose", "Jose", true, 16)
        val other = Member("other0000001", "ann", "Ann", false, 3, "OtherAsset000001", "https://cdn.caper.chat/assets/OtherAsset000001?sig=o")
        val author = ChatAuthor(own.id, "Jose", false, 16)
        val otherAuthor = ChatAuthor(other.id, "Ann", false, 3, avatarAsset = other.avatarAsset, avatarUrl = other.avatarUrl)
        val reply = MessageReply("message00000", author, "earlier", false)
        val message = ChatMessage("message00001", "channel00001", "7", otherAuthor, ChatContent(1, "text", "hi"), "now", "client",
            pin = MessagePin(author, "now"), thread = ThreadSummary(1, listOf(author, otherAuthor), "7"), replyTo = reply)
        val mine = message.copy(id = "message00002", author = author, replyTo = null, pin = null, thread = null)
        val self = DirectConversation("direct000001", DirectPeer(own.id, "jose", "Jose", 16), "0", "0")
        val state = AppUiState(
            account = Account(own.id, "jose", "Jose", avatarId = 16, avatarAsset = asset, avatarUrl = url),
            selectedSpace = SpaceDetail(Space("space0000001", "Studio", own.id), emptyList(), listOf(own, other)),
            messages = listOf(message, mine), pinnedMessages = listOf(mine), directConversations = listOf(self),
            pendingMessage = PendingMessageUi("pending", "draft", author, "now"),
        )

        val shown = state.withAccountProfile()
        val members = shown.selectedSpace!!.members
        assertEquals(asset, members[0].avatarAsset)
        assertEquals(url, members[0].avatarUrl)
        assertEquals(16, members[0].avatarId)
        assertEquals(other, members[1])
        assertEquals(otherAuthor, shown.messages[0].author)
        assertEquals(asset, shown.messages[0].pin!!.author.avatarAsset)
        assertEquals(listOf(asset, other.avatarAsset), shown.messages[0].thread!!.participants.map { it.avatarAsset })
        assertEquals(url, shown.messages[0].replyTo!!.author.avatarUrl)
        assertEquals(asset, shown.messages[1].author.avatarAsset)
        assertEquals(asset, shown.pinnedMessages[0].author.avatarAsset)
        assertEquals(asset, shown.directConversations[0].peer.avatarAsset)
        assertEquals(url, shown.pendingMessage!!.author!!.avatarUrl)
        // History, cursors and text are untouched.
        assertEquals(state.messages.map { it.content }, shown.messages.map { it.content })

        // A snapshot already naming this picture keeps its own (perhaps fresher) URL.
        val fresher = "https://cdn.caper.chat/assets/$asset?exp=1790086400&sig=f"
        val signedLater = state.copy(messages = listOf(mine.copy(author = author.copy(avatarAsset = asset, avatarUrl = fresher))))
        assertEquals(fresher, signedLater.withAccountProfile().messages[0].author.avatarUrl)

        // Removed: the same saved mascot everywhere, other people untouched.
        val removed = state.copy(
            account = state.account!!.copy(avatarAsset = null, avatarUrl = null),
            messages = listOf(mine.copy(author = author.copy(avatarAsset = asset, avatarUrl = url))),
        ).withAccountProfile()
        assertNull(removed.messages[0].author.avatarAsset)
        assertEquals(16, removed.messages[0].author.avatarId)
        assertEquals(other, removed.selectedSpace!!.members[1])
    }
}
