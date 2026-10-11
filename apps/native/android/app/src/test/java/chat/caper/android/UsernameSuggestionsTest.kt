package chat.caper.android

import chat.caper.android.model.Member
import chat.caper.android.model.Person
import org.junit.Assert.*
import org.junit.Test

class UsernameSuggestionsTest {
    private fun person(username: String, displayName: String = username) = MentionCandidate(username, displayName)
    private fun member(username: String, displayName: String = username, owner: Boolean = false) = Member("id_$username", username, displayName, owner)
    private val people = listOf(
        person("xal", "Someone"), person("zed", "Al Green"), person("albert"), person("al"),
        person("bob", "Mary Alvarez"), person("alan"), person("carol"),
    )

    @Test fun `query is trimmed, drops one leading at sign and is lowercase`() {
        assertEquals("al", usernameQuery("  @AL "))
        assertEquals("@al", usernameQuery("@@al"))
        assertEquals("", usernameQuery(" @ "))
    }

    @Test fun `fields rank like mentions and cap at six`() {
        assertEquals(listOf("al", "alan", "albert", "bob", "zed", "xal"), usernameSuggestions(people, "@AL").map { it.username })
        assertEquals(mentionSuggestions(MentionSource(people, specials = false), "al"), usernameSuggestions(people, " al "))
        assertEquals(listOf("al"), usernameSuggestions(people, "al", limit = 1).map { it.username })
        assertEquals(6, usernameSuggestions((1..9).map { person("user$it") }, "user").size)
        assertTrue(usernameSuggestions(people, "qq").isEmpty())
    }

    @Test fun `an empty query lists nobody`() {
        listOf("", "  ", "@", " @ ").forEach { assertTrue(it, usernameSuggestions(people, it).isEmpty()) }
    }

    @Test fun `shared ranking matches display-name words and sorts ties by lowercase username`() {
        val ranked = rankPeople(listOf(person("Zoe", "Ann Lee"), person("amy", "Lee Park"), person("lee"), person("kalee")), "LEE")
        assertEquals(listOf("lee", "amy", "Zoe", "kalee"), ranked.map { it.username })
        assertEquals(listOf("amy", "lee", "Zoe"), rankPeople(listOf(person("Zoe"), person("lee"), person("amy")), "").map { it.username })
    }

    @Test fun `space invites suggest known people outside the space and its pending invitations, by id`() {
        val known = listOf(Person("id_alex", "alex", "Alex Doe", 3), Person("id_bob", "bob", "Bob"), Person("id_cy", "cy", "Cy"), Person("id_dee", "dee", "Dee"))
        // Matched by id: a member whose username changed is still excluded.
        val members = listOf(member("me", owner = true), Member("id_bob", "bobby", "Bob", false))
        val candidates = spaceInviteCandidates(known, members, listOf(member("cy")))
        assertEquals(listOf(MentionCandidate("alex", "Alex Doe", avatarId = 3), MentionCandidate("dee", "Dee")), candidates)
        assertTrue(spaceInviteCandidates(null, members, emptyList()).isEmpty())
    }

    @Test fun `private channels suggest space members without you, current access or pending invitations`() {
        val members = listOf(member("me", owner = true), Member("id_ann", "ann", "Ann", false, 9), member("bob"), member("cy"), member("dee"))
        val candidates = channelGrantCandidates(members, "id_me", grants = listOf(member("bob")), invitations = listOf(Member("id_cy", "renamed", "Cy", false)))
        assertEquals(listOf(MentionCandidate("ann", "Ann", avatarId = 9), MentionCandidate("dee", "dee")), candidates)
        assertEquals(listOf("ann", "bob", "cy", "dee"), channelGrantCandidates(members, "id_me", emptyList(), emptyList()).map { it.username })
        assertTrue(channelGrantCandidates(emptyList(), "id_me", emptyList(), emptyList()).isEmpty())
    }

    @Test fun `new DMs suggest known people, never you`() {
        val known = listOf(Person("id_alex", "alex", "Alex Doe", 3), Person("id_me", "me", "Me"), Person("id_bob", "bob", "Bob"))
        assertEquals(listOf(MentionCandidate("alex", "Alex Doe", avatarId = 3), MentionCandidate("bob", "Bob")), directCandidates(known, "id_me"))
        assertEquals(3, directCandidates(known, null).size)
        assertTrue(directCandidates(null, "id_me").isEmpty())
        assertEquals(listOf("alex"), usernameSuggestions(directCandidates(known, "id_me"), "doe").map { it.username })
    }
}
