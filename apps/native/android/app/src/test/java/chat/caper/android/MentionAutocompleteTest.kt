package chat.caper.android

import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.input.TextFieldValue
import chat.caper.android.model.Account
import chat.caper.android.model.AppUiState
import chat.caper.android.model.Channel
import chat.caper.android.model.ChatContent
import chat.caper.android.model.ChatAuthor
import chat.caper.android.model.DirectConversation
import chat.caper.android.model.DirectPeer
import chat.caper.android.model.Member
import chat.caper.android.model.MessageMention
import chat.caper.android.model.PeopleList
import chat.caper.android.model.Person
import chat.caper.android.model.Space
import chat.caper.android.model.SpaceDetail
import kotlinx.serialization.json.Json
import org.junit.Assert.*
import org.junit.Test

class MentionAutocompleteTest {
    private fun atEnd(text: String) = TextFieldValue(text, TextRange(text.length))
    private fun person(username: String, displayName: String = username) = MentionCandidate(username, displayName)
    private fun space(vararg people: MentionCandidate) = MentionSource(people.toList(), specials = true)
    private fun user(username: String, id: String = "id_$username") = MessageMention("user", id, username)
    private val everyone = MessageMention("everyone")
    private val here = MessageMention("here")

    @Test fun `token requires the start rule and a whole name run at the caret`() {
        listOf("bob@ali", "x/@ali", "@@ali", "a,@ali", "a:@ali").forEach { assertNull(it, mentionToken(atEnd(it))) }
        assertEquals(MentionToken(0, 4, "ali"), mentionToken(atEnd("@ali")))
        assertEquals("ali", mentionToken(atEnd("hi (@ali"))?.query)
        assertEquals("ali", mentionToken(atEnd("[@ali"))?.query)
        assertEquals("Al_1", mentionToken(atEnd("line\n{@Al_1"))?.query)
        assertEquals("", mentionToken(atEnd("hello @"))?.query)
        assertEquals("ali", mentionToken(TextFieldValue("@ali bob", TextRange(4)))?.query)
        assertEquals("ali", mentionToken(TextFieldValue("@ali!", TextRange(4)))?.query)
    }

    @Test fun `token rejects caret mid name, long runs, selections and composition`() {
        assertNull(mentionToken(TextFieldValue("@alice", TextRange(3))))
        assertNull(mentionToken(TextFieldValue("@ali@", TextRange(4))))
        assertNotNull(mentionToken(atEnd("@" + "a".repeat(32))))
        assertNull(mentionToken(atEnd("@" + "a".repeat(33))))
        assertNull(mentionToken(TextFieldValue("@ali", TextRange(1, 4))))
        assertNull(mentionToken(TextFieldValue("@ali", TextRange(4), TextRange(1, 4))))
    }

    @Test fun `emoji and mention tokens are never both active`() {
        listOf("@:sm", ":@al", "hi :sm", "hi @al", "(:x", "(@x").map(::atEnd).forEach {
            assertFalse(it.text, emojiToken(it) != null && mentionToken(it) != null)
        }
    }

    @Test fun `ranking orders exact, prefix, display-name word and contains, ties by username`() {
        val source = MentionSource(listOf(
            person("xal", "Someone"), person("zed", "Al Green"), person("albert"), person("al"),
            person("bob", "Mary Alvarez"), person("alan"), person("carol"),
        ), specials = false)
        assertEquals(listOf("al", "alan", "albert", "bob", "zed", "xal"), mentionSuggestions(source, "AL").map { it.username })
        assertEquals(listOf("al", "alan", "albert", "bob", "carol", "xal"), mentionSuggestions(source, "").map { it.username })
        assertTrue(mentionSuggestions(source, "qq").isEmpty())
    }

    @Test fun `specials follow members, keep their slots and only appear in space channels`() {
        val people = (1..8).map { person("user$it") }
        val all = mentionSuggestions(MentionSource(people, specials = true), "")
        assertEquals(listOf("user1", "user2", "user3", "user4", "everyone", "here"), all.map { it.username })
        assertTrue(all.takeLast(2).all { it.special })
        assertEquals(listOf("everyone"), mentionSuggestions(space(person("bob")), "ev").map { it.username })
        assertEquals(listOf("here"), mentionSuggestions(space(person("bob")), "HE").map { it.username })
        assertEquals(listOf("everybody", "everyone"), mentionSuggestions(space(person("everybody")), "every").map { it.username })
        assertEquals(6, mentionSuggestions(MentionSource(people, specials = false), "").size)
        assertTrue(mentionSuggestions(MentionSource(emptyList(), specials = false), "ev").isEmpty())
    }

    @Test fun `candidates come from space members or DM people, never yourself`() {
        val me = Account("me000000000a", "me", "Me")
        val members = listOf(Member("me000000000a", "me", "Me", true), Member("bob00000000a", "bob", "Bob B", false, 7))
        val channel = Channel("channel00001", "space0000001", "general", private = false)
        val detail = SpaceDetail(Space("space0000001", "Space"), listOf(channel), members)
        val inSpace = mentionSource(AppUiState(account = me, selectedSpace = detail, selectedChannel = channel))
        assertEquals(listOf(MentionCandidate("bob", "Bob B", avatarId = 7)), inSpace.people)
        assertTrue(inSpace.specials)
        val notLoaded = mentionSource(AppUiState(account = me, selectedChannel = channel))
        assertEquals(MentionSource(emptyList(), true), notLoaded)

        val dm = DirectConversation("direct000001", DirectPeer("bob00000000a", "bob", "Bob B"), "0", "0")
        val notes = DirectConversation("direct000002", DirectPeer("me000000000a", "me", "Me"), "0", "0")
        fun direct(conversation: DirectConversation, people: List<Person>? = null) = mentionSource(AppUiState(
            account = me, selectedSpace = detail, directConversations = listOf(dm, notes), selectedDirectId = conversation.id,
            selectedChannel = Channel(conversation.id, "", conversation.peer.displayName, private = true, direct = true), people = people,
        ))
        // Before `/api/people` loads (or when it fails): the DM peer, nobody in self-notes.
        assertEquals(MentionSource(listOf(MentionCandidate("bob", "Bob B")), false), direct(dm))
        assertEquals(MentionSource(emptyList(), false), direct(notes))

        // Once loaded, every DM (self-notes too) suggests the people list without you or specials.
        val people = listOf(Person("alex0000000a", "alex", "Alex Doe", 3), Person("me000000000a", "me", "Me"), Person("bob00000000a", "bob", "Bob B"))
        val expected = MentionSource(listOf(MentionCandidate("alex", "Alex Doe", avatarId = 3), MentionCandidate("bob", "Bob B")), false)
        assertEquals(expected, direct(dm, people))
        assertEquals(expected, direct(notes, people))
        assertEquals(MentionSource(emptyList(), false), direct(dm, emptyList()))
        assertEquals(listOf("alex"), mentionSuggestions(direct(dm, people), "do").map { it.username })
        assertTrue(mentionSuggestions(direct(dm, people), "every").isEmpty())
        // Space channels ignore the people list.
        assertEquals(inSpace, mentionSource(AppUiState(account = me, selectedSpace = detail, selectedChannel = channel, people = people)))
    }

    @Test fun `people response decodes with optional avatars`() {
        val json = Json { ignoreUnknownKeys = true }
        val decoded = json.decodeFromString<PeopleList>("""{"people":[{"id":"alex0000000a","username":"alex","displayName":"Alex","avatarId":null},
            {"id":"bob00000000a","username":"bob","displayName":"Bob","avatarId":12,"future":true},{"id":"cy000000000a","username":"cy","displayName":"Cy"}]}""")
        assertEquals(listOf(Person("alex0000000a", "alex", "Alex"), Person("bob00000000a", "bob", "Bob", 12), Person("cy000000000a", "cy", "Cy")), decoded.people)
        assertEquals(emptyList<Person>(), json.decodeFromString<PeopleList>("""{"people":[]}""").people)
    }

    @Test fun `insertion adds a trailing space, keeps Unicode around it and places the caret`() {
        val original = TextFieldValue("👩‍💻 hi @bo, 🚀 later", TextRange("👩‍💻 hi @bo".length))
        val result = insertMention(original, mentionToken(original)!!, "bob")!!
        assertEquals("👩‍💻 hi @bob , 🚀 later", result.text)
        assertEquals("👩‍💻 hi @bob ".length, result.selection.start)
        assertTrue(result.selection.collapsed)
        assertEquals("@everyone ", insertMention(atEnd("@"), mentionToken(atEnd("@"))!!, "everyone")!!.text)
    }

    @Test fun `insertion enforces the scalar limit`() {
        val fits = atEnd("😀".repeat(3994) + " @b")
        assertEquals(4000, insertMention(fits, mentionToken(fits)!!, "bob")!!.text.let { it.codePointCount(0, it.length) })
        val over = atEnd("😀".repeat(3995) + " @b")
        assertNull(insertMention(over, mentionToken(over)!!, "bob"))
    }

    @Test fun `message tokens follow the grammar`() {
        assertEquals(listOf(MentionSpan(3, 9, "alice")), mentionSpans("hi @Alice!"))
        assertEquals(listOf("alice", "bob", "carol"), mentionSpans("(@alice) [@bob] {@carol}").map { it.name })
        assertEquals(listOf("alice"), mentionSpans("@Alice's").map { it.name })
        assertEquals(listOf("bob"), mentionSpans("🙂 @bob").map { it.name })
        listOf("bob@alice.com", "x/@alice", "@@alice", "@", "@ alice", "a,@alice", "@" + "a".repeat(33)).forEach {
            assertTrue(it, mentionSpans(it).isEmpty())
        }
        assertEquals(listOf("alice"), mentionSpans("@alice@bob").map { it.name })
        assertEquals(listOf("a".repeat(32)), mentionSpans("@" + "a".repeat(32)).map { it.name })
    }

    @Test fun `highlights only resolved names, case-insensitively, ignoring unknown types`() {
        val text = "@Alice @bob @EVERYONE @here @carol"
        assertEquals(listOf("alice"), highlightedMentions(text, listOf(user("alice"))).map { it.name })
        assertEquals(listOf("alice", "everyone"), highlightedMentions(text, listOf(user("ALICE"), everyone)).map { it.name })
        assertEquals(listOf("here"), highlightedMentions(text, listOf(here)).map { it.name })
        assertTrue(highlightedMentions(text, emptyList()).isEmpty())
        assertTrue(highlightedMentions(text, listOf(MessageMention("role", "x", "carol"), MessageMention("user", "id"))).isEmpty())
        assertEquals(listOf(MentionSpan(0, 6, "alice")), highlightedMentions("@alice", listOf(user("alice"))))
    }

    @Test fun `only person pills resolve to a user entry`() {
        val entries = listOf(user("alice"), everyone, here, MessageMention("role", "x", "team"))
        val spans = mentionSpans("@Alice @everyone @here @team @bob")
        assertEquals(listOf(user("alice"), null, null, null, null), spans.map { mentionedUser(it, entries) })
    }

    @Test fun `mention card resolves members, then people, then DM peers, then unknown`() {
        val me = Account("me000000000a", "me", "Me Myself", avatarId = 9)
        val space = SpaceDetail(Space("space0000001", "Space"), emptyList(), listOf(
            Member("me000000000a", "me", "Me Myself", true, 9), Member("maya0000000a", "maya", "Maya Member", false, 4),
        ))
        val people = listOf(Person("maya0000000a", "maya", "Maya Person", 5), Person("alex0000000a", "alex", "Alex Person", null))
        val dms = listOf(
            DirectConversation("direct000001", DirectPeer("alex0000000a", "alex", "Alex Peer"), "0", "0"),
            DirectConversation("direct000002", DirectPeer("sam00000000a", "sam", "Sam Peer"), "0", "0"),
        )
        val state = AppUiState(account = me, selectedSpace = space, people = people, directConversations = dms)

        assertEquals(MentionCard("maya0000000a", "maya", "Maya Member", 4), mentionCard("maya0000000a", "maya", state))
        assertEquals(MentionCard("alex0000000a", "alex", "Alex Person", null), mentionCard("alex0000000a", "alex", state))
        assertEquals(MentionCard("sam00000000a", "sam", "Sam Peer", null), mentionCard("sam00000000a", "sam", state))
        assertEquals(MentionCard("maya0000000a", "maya", "Maya Person", 5), mentionCard("maya0000000a", "maya", state.copy(selectedSpace = null)))
        assertEquals(MentionCard("alex0000000a", "alex", "Alex Peer", null), mentionCard("alex0000000a", "alex", state.copy(people = null)))

        // Tagged, but nothing shared: `@username` title, no second line, still messageable by username.
        val unknown = mentionCard("zed00000000a", "zed", state)
        assertEquals(MentionCard("zed00000000a", "zed", null, null, self = false), unknown)
        assertEquals("@zed", unknown.title)
        assertNull(unknown.subtitle)
        assertEquals("Maya Member", mentionCard("maya0000000a", "maya", state).title)
        assertEquals("@maya", mentionCard("maya0000000a", "maya", state).subtitle)
        // An entry without an id never matches by name alone.
        assertEquals(MentionCard(null, "maya", null), mentionCard(null, "maya", state))
    }

    @Test fun `mention card marks you by id, or by username when the entry has no id`() {
        val me = Account("me000000000a", "me", "Me Myself", avatarId = 9)
        val member = SpaceDetail(Space("space0000001", "Space"), emptyList(), listOf(Member("me000000000a", "me", "Me Myself", true, 9)))
        assertTrue(mentionCard("me000000000a", "me", AppUiState(account = me, selectedSpace = member)).self)
        // People lists exclude you; your own account still fills the card.
        assertEquals(MentionCard("me000000000a", "me", "Me Myself", 9, self = true), mentionCard("me000000000a", "me", AppUiState(account = me)))
        assertTrue(mentionCard(null, "ME", AppUiState(account = me)).self)
        assertFalse(mentionCard("other000000a", "me", AppUiState(account = me)).self)
        assertFalse(mentionCard("me000000000a", "me", AppUiState()).self)
        assertTrue(mentionCard("me000000000a", "me", AppUiState(chatAuthorId = "me000000000a")).self)
    }

    @Test fun `pinner profiles match IDs instead of names and do not invent usernames`() {
        val author = ChatAuthor("pinner000001", "Same Name", false, 7)
        val members = listOf(
            Member("writer000001", "wrong_person", "Same Name", false, 2),
            Member("pinner000001", "renamed_pinner", "Current Name", false, null),
        )
        val state = AppUiState(chatAuthorId = "writer000001", selectedSpace = SpaceDetail(Space("space0000001", "Space"), emptyList(), members))
        assertEquals(MentionCard(author.id, "renamed_pinner", "Current Name", null, false), authorCard(author, state))
        val unknown = authorCard(author, state.copy(selectedSpace = null))
        assertEquals(MentionCard(author.id, null, "Same Name", 7, false), unknown)
        assertNull(unknown.subtitle)
        assertTrue(authorCard(author, AppUiState(chatAuthorId = author.id)).self)
    }

    @Test fun `mentions me by id, or by everyone and here from someone else`() {
        assertTrue(mentionsMe(listOf(user("me", "me_id")), "other", "me_id"))
        assertFalse(mentionsMe(listOf(user("me", "not_me")), "other", "me_id"))
        assertTrue(mentionsMe(listOf(everyone), "other", "me_id"))
        assertTrue(mentionsMe(listOf(here), "other", "me_id"))
        assertFalse(mentionsMe(listOf(everyone, here), "me_id", "me_id"))
        assertTrue(mentionsMe(listOf(everyone, user("me", "me_id")), "me_id", "me_id"))
        assertFalse(mentionsMe(listOf(MessageMention("role", "me_id")), "other", "me_id"))
        assertFalse(mentionsMe(listOf(everyone), "other", null))
        assertFalse(mentionsMe(emptyList(), "other", "me_id"))
    }

    @Test fun `content mentions are optional and tolerate unknown or malformed entries`() {
        val json = Json { ignoreUnknownKeys = true }
        assertEquals(emptyList<MessageMention>(), json.decodeFromString<ChatContent>("""{"version":1,"type":"text","text":"hi"}""").mentions)
        val decoded = json.decodeFromString<ChatContent>("""{"version":1,"type":"text","text":"hey @alice @everyone","mentions":[
            {"type":"user","id":"abcdefghijkl","username":"alice"},{"type":"everyone"},{"type":"role","id":42,"extra":{"a":1}},
            {"id":"no-type"},"text",{"type":"user","id":null,"username":"bob"}]}""").mentions
        assertEquals(listOf(
            MessageMention("user", "abcdefghijkl", "alice"), MessageMention("everyone"), MessageMention("role"),
            MessageMention("user", null, "bob"),
        ), decoded)
        assertEquals(listOf("alice", "everyone"), highlightedMentions("hey @alice @everyone", decoded).map { it.name })
    }
}
